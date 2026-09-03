#!/usr/bin/env python3
"""Read-only Cargo storage and Git worktree audit.

This script never deletes files, changes Git state, fetches, or invokes Cargo. It reports
which directories appear to be Cargo target roots; a human must revalidate and authorize
any later cleanup.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple


AUDIT_VERSION = 1
CACHE_SIGNATURE = "Signature: 8a477f597d28d172789f06886806bc55"


def run_bytes(args: Sequence[str], cwd: Optional[Path] = None) -> Tuple[int, bytes, bytes]:
    try:
        proc = subprocess.run(
            list(args),
            cwd=str(cwd) if cwd else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        return 127, b"", str(error).encode("utf-8", errors="replace")
    return proc.returncode, proc.stdout, proc.stderr


def git(args: Sequence[str], cwd: Path) -> Tuple[int, bytes, bytes]:
    return run_bytes(["git", "--no-optional-locks", "-C", str(cwd), *args])


def decode(value: bytes) -> str:
    return value.decode("utf-8", errors="surrogateescape")


def human_bytes(value: Optional[int]) -> str:
    if value is None:
        return "unknown"
    amount = float(value)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if amount < 1024.0 or unit == "TiB":
            return f"{amount:.1f} {unit}"
        amount /= 1024.0
    return f"{amount:.1f} TiB"


def du_bytes(path: Path) -> Optional[int]:
    code, out, _ = run_bytes(["du", "-sk", str(path)])
    if code != 0:
        return None
    try:
        return int(decode(out).split(None, 1)[0]) * 1024
    except (IndexError, ValueError):
        return None


def filesystem_snapshot(path: Path) -> Dict[str, int]:
    usage = shutil.disk_usage(path)
    stat = os.statvfs(path)
    return {
        "bytes_total": usage.total,
        "bytes_used": usage.used,
        "bytes_free": usage.free,
        "inodes_total": stat.f_files,
        "inodes_free": stat.f_ffree,
    }


def nearest_existing_parent(path: Path) -> Path:
    candidate = path
    while not candidate.exists() and candidate != candidate.parent:
        candidate = candidate.parent
    return candidate


def memory_snapshot() -> Dict[str, Optional[int]]:
    result: Dict[str, Optional[int]] = {
        "bytes_total": None,
        "bytes_available_estimate": None,
        "swap_bytes_total": None,
        "swap_bytes_used": None,
    }
    code, out, _ = run_bytes(["sysctl", "-n", "hw.memsize"])
    if code == 0:
        try:
            result["bytes_total"] = int(decode(out).strip())
        except ValueError:
            pass

    code, out, _ = run_bytes(["vm_stat"])
    if code == 0:
        text = decode(out)
        page_match = re.search(r"page size of (\d+) bytes", text)
        if page_match:
            page_size = int(page_match.group(1))
            pages = 0
            for label in ("Pages free", "Pages inactive", "Pages speculative"):
                match = re.search(rf"^{re.escape(label)}:\s+(\d+)\.", text, re.MULTILINE)
                if match:
                    pages += int(match.group(1))
            result["bytes_available_estimate"] = pages * page_size

    code, out, _ = run_bytes(["sysctl", "-n", "vm.swapusage"])
    if code == 0:
        text = decode(out)
        total = re.search(r"total = ([0-9.]+)([MG])", text)
        used = re.search(r"used = ([0-9.]+)([MG])", text)
        multipliers = {"M": 1024**2, "G": 1024**3}
        if total:
            result["swap_bytes_total"] = int(float(total.group(1)) * multipliers[total.group(2)])
        if used:
            result["swap_bytes_used"] = int(float(used.group(1)) * multipliers[used.group(2)])

    if result["bytes_total"] is None and Path("/proc/meminfo").is_file():
        try:
            entries = {}
            for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
                key, _, value = line.partition(":")
                match = re.match(r"\s*(\d+)\s+kB", value)
                if match:
                    entries[key] = int(match.group(1)) * 1024
            result["bytes_total"] = entries.get("MemTotal")
            result["bytes_available_estimate"] = entries.get("MemAvailable")
            result["swap_bytes_total"] = entries.get("SwapTotal")
            swap_free = entries.get("SwapFree")
            if result["swap_bytes_total"] is not None and swap_free is not None:
                result["swap_bytes_used"] = result["swap_bytes_total"] - swap_free
        except OSError:
            pass
    return result


def parse_worktrees(repo: Path) -> List[Dict[str, Any]]:
    code, out, err = git(["worktree", "list", "--porcelain", "-z"], repo)
    if code != 0:
        raise RuntimeError(f"git worktree list failed: {decode(err).strip()}")

    entries: List[Dict[str, Any]] = []
    current: Optional[Dict[str, Any]] = None
    for raw in out.split(b"\0"):
        if not raw:
            if current:
                entries.append(current)
                current = None
            continue
        key, _, value = raw.partition(b" ")
        field = decode(key)
        text = decode(value)
        if field == "worktree":
            if current:
                entries.append(current)
            current = {"path": text}
        elif current is not None:
            if field in {"detached", "bare", "prunable"}:
                current[field] = True
            elif field == "locked":
                current["locked"] = True
                if text:
                    current["locked_reason"] = text
            else:
                current[field] = text
    if current:
        entries.append(current)
    return entries


def normalized_path(path: Path) -> Path:
    """Return an absolute, lexically normalized path without following symlinks."""

    return Path(os.path.abspath(os.fspath(path)))


def path_is_within(path: Path, root: Path) -> bool:
    """Return whether path is root or one of its descendants."""

    candidate = normalized_path(path)
    boundary = normalized_path(root)
    try:
        candidate.relative_to(boundary)
    except ValueError:
        return False
    return True


def status_snapshot(worktree: Path, target_roots: Sequence[Path]) -> Dict[str, Any]:
    code, out, err = git(
        [
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
        worktree,
    )
    if code != 0:
        return {
            "error": decode(err).strip(),
            "protected_reasons": ["worktree status inspection failed"],
        }

    normalized_targets = sorted(
        {normalized_path(root) for root in target_roots}, key=lambda path: str(path)
    )
    counts = {
        "tracked": 0,
        "unmerged": 0,
        "untracked": 0,
        "ignored": 0,
        "ignored_within_target": 0,
        "ignored_other": 0,
    }
    samples: Dict[str, List[str]] = {key: [] for key in counts}
    branch: Dict[str, Any] = {}
    for raw in out.split(b"\0"):
        if not raw:
            continue
        item = decode(raw)
        if item.startswith("# branch.oid "):
            branch["head"] = item[len("# branch.oid ") :]
        elif item.startswith("# branch.head "):
            branch["name"] = item[len("# branch.head ") :]
        elif item.startswith("# branch.upstream "):
            branch["upstream"] = item[len("# branch.upstream ") :]
        elif item.startswith("# branch.ab "):
            match = re.match(r"# branch\.ab \+(\d+) -(\d+)", item)
            if match:
                branch["ahead"] = int(match.group(1))
                branch["behind"] = int(match.group(2))
        elif item.startswith(("1 ", "2 ")):
            counts["tracked"] += 1
            if len(samples["tracked"]) < 5:
                samples["tracked"].append(item)
        elif item.startswith("u "):
            counts["unmerged"] += 1
            if len(samples["unmerged"]) < 5:
                samples["unmerged"].append(item)
        elif item.startswith("? "):
            counts["untracked"] += 1
            if len(samples["untracked"]) < 5:
                samples["untracked"].append(item[2:])
        elif item.startswith("! "):
            counts["ignored"] += 1
            if len(samples["ignored"]) < 5:
                samples["ignored"].append(item[2:])
            ignored_path = normalized_path(worktree / item[2:])
            category = (
                "ignored_within_target"
                if any(path_is_within(ignored_path, root) for root in normalized_targets)
                else "ignored_other"
            )
            counts[category] += 1
            if len(samples[category]) < 5:
                samples[category].append(item[2:])

    protected = []
    if counts["tracked"] or counts["unmerged"]:
        protected.append("tracked or unmerged changes")
    if counts["untracked"]:
        protected.append("untracked files")
    if counts["ignored_other"]:
        protected.append("ignored files may contain review evidence")
    if branch.get("ahead", 0):
        protected.append("commits ahead of upstream")
    if not branch.get("upstream"):
        protected.append("no upstream recorded")
    return {
        "branch": branch,
        "status_counts": counts,
        "samples": samples,
        "target_roots": [str(path) for path in normalized_targets],
        "protected_reasons": protected,
    }


def configured_target(worktree: Path) -> Optional[Tuple[Path, str, str]]:
    env_target = os.environ.get("CARGO_TARGET_DIR")
    if env_target:
        candidate = Path(env_target)
        if not candidate.is_absolute():
            candidate = worktree / candidate
        return candidate, "CARGO_TARGET_DIR environment", "explicit-environment"

    for filename in (worktree / ".cargo" / "config.toml", worktree / ".cargo" / "config"):
        if not filename.is_file():
            continue
        try:
            content = filename.read_text(encoding="utf-8")
        except OSError:
            continue
        in_build = False
        for line in content.splitlines():
            stripped = line.split("#", 1)[0].strip()
            if stripped.startswith("[") and stripped.endswith("]"):
                in_build = stripped == "[build]"
                continue
            match = re.match(r"target-dir\s*=\s*([\"'])(.+?)\1\s*$", stripped)
            if in_build and match:
                candidate = Path(match.group(2))
                if not candidate.is_absolute():
                    candidate = worktree / candidate
                return (
                    candidate,
                    f"{filename} build.target-dir",
                    "heuristic-config-parse; Cargo precedence not fully resolved",
                )
    return None


def discover_target_roots(worktrees: Iterable[Dict[str, Any]]) -> List[Dict[str, Any]]:
    found: Dict[str, Dict[str, Any]] = {}
    for entry in worktrees:
        worktree = Path(entry["path"])
        configured = configured_target(worktree)
        candidates = [configured] if configured else []
        candidates.append(
            (worktree / "target", "conventional worktree target", "exact-convention")
        )
        for candidate, source, confidence in candidates:
            lexical = candidate.absolute()
            canonical = lexical.resolve(strict=False)
            key = str(canonical)
            if key in found:
                if str(worktree) not in found[key]["worktrees"]:
                    found[key]["worktrees"].append(str(worktree))
                if source not in found[key]["sources"]:
                    found[key]["sources"].append(source)
                if str(lexical) not in found[key]["lexical_paths"]:
                    found[key]["lexical_paths"].append(str(lexical))
                if confidence not in found[key]["discovery_confidence"]:
                    found[key]["discovery_confidence"].append(confidence)
            elif lexical.exists() or source != "conventional worktree target":
                found[key] = {
                    "path": key,
                    "sources": [source],
                    "worktrees": [str(worktree)],
                    "lexical_paths": [str(lexical)],
                    "discovery_confidence": [confidence],
                }
    return list(found.values())


def target_roots_by_worktree(
    discoveries: Iterable[Dict[str, Any]],
) -> Dict[str, List[Path]]:
    """Index canonical and lexical target roots by their owning worktree."""

    indexed: Dict[str, Dict[str, Path]] = {}
    for discovery in discoveries:
        roots = {
            str(normalized_path(Path(path))): normalized_path(Path(path))
            for path in [discovery["path"], *discovery["lexical_paths"]]
        }
        for owner in discovery["worktrees"]:
            owner_key = str(normalized_path(Path(owner)))
            indexed.setdefault(owner_key, {}).update(roots)
    return {
        owner: [roots[key] for key in sorted(roots)]
        for owner, roots in indexed.items()
    }


def cache_tag_snapshot(target: Path) -> Dict[str, Any]:
    tag = target / "CACHEDIR.TAG"
    result = {
        "path": str(tag),
        "present": tag.exists() or tag.is_symlink(),
        "symlink": tag.is_symlink(),
        "regular_file": False,
        "first_line": None,
        "exact": False,
    }
    if not tag.exists() or tag.is_symlink() or not tag.is_file():
        return result
    result["regular_file"] = True
    try:
        with tag.open("r", encoding="utf-8", errors="replace") as handle:
            first_line = handle.readline().rstrip("\r\n")
    except OSError:
        return result
    result["first_line"] = first_line
    result["exact"] = first_line == CACHE_SIGNATURE
    return result


def has_symlink_component(path: Path) -> bool:
    absolute = path.absolute()
    current = Path(absolute.anchor)
    for part in absolute.parts[1:]:
        current = current / part
        if current.is_symlink():
            return True
    return False


def registered_protected_paths(worktrees: Iterable[Dict[str, Any]]) -> Dict[str, List[str]]:
    roots = []
    git_metadata = []
    for entry in worktrees:
        root = Path(entry["path"]).resolve()
        roots.append(str(root))
        for argument in ("--absolute-git-dir", "--git-common-dir"):
            code, out, _ = git(["rev-parse", argument], root)
            if code != 0:
                continue
            value = Path(decode(out).strip())
            if not value.is_absolute():
                value = root / value
            resolved = str(value.resolve(strict=False))
            if resolved not in git_metadata:
                git_metadata.append(resolved)
    return {"worktree_roots": roots, "git_metadata": git_metadata}


def unsafe_target_reasons(
    target: Path,
    discovery: Dict[str, Any],
    protected_paths: Dict[str, List[str]],
) -> List[str]:
    reasons = []
    root = Path(target.anchor).resolve()
    home = Path.home().resolve()
    if target == root:
        reasons.append("target resolves to the filesystem root")
    if target == home:
        reasons.append("target resolves to the user home directory")
    owners = {str(Path(owner).resolve()) for owner in discovery["worktrees"]}
    for root_text in protected_paths["worktree_roots"]:
        worktree = Path(root_text)
        if target == worktree:
            reasons.append(f"target is a registered worktree root: {worktree}")
        elif target in worktree.parents:
            reasons.append(f"target is an ancestor of registered worktree: {worktree}")
        elif worktree in target.parents and root_text not in owners:
            reasons.append(f"target is nested in non-owner worktree: {worktree}")
    for metadata_text in protected_paths["git_metadata"]:
        metadata = Path(metadata_text)
        if (
            target == metadata
            or target in metadata.parents
            or metadata in target.parents
        ):
            reasons.append(f"target overlaps registered Git metadata: {metadata}")
    for lexical in discovery["lexical_paths"]:
        if has_symlink_component(Path(lexical)):
            reasons.append(f"discovered path contains a symlink component: {lexical}")
    return sorted(set(reasons))


def running_rust_processes() -> Dict[str, Any]:
    code, out, _ = run_bytes(["ps", "-axo", "pid=,rss=,comm="])
    if code != 0:
        return {"ok": False, "processes": [], "error": f"ps exited {code}"}
    found = []
    for line in decode(out).splitlines():
        match = re.match(r"\s*(\d+)\s+(\d+)\s+(.+)$", line)
        if not match:
            continue
        name = Path(match.group(3).strip()).name
        if name in {
            "cargo",
            "rustc",
            "rustdoc",
            "sccache",
            "hex_game",
            "clang",
            "clang++",
            "cc",
            "c++",
            "ld",
            "ld64",
            "ld.lld",
            "lld",
            "mold",
            "collect2",
            "wasm-ld",
        }:
            found.append(
                {
                    "pid": int(match.group(1)),
                    "rss_bytes": int(match.group(2)) * 1024,
                    "process": name,
                }
            )
    return {"ok": True, "processes": found, "error": None}


def target_snapshot(
    discovery: Dict[str, Any],
    active: List[Dict[str, Any]],
    process_scan_ok: bool,
    worktree_statuses: Dict[str, Dict[str, Any]],
    repo_filesystem_device: int,
    protected_paths: Dict[str, List[str]],
) -> Dict[str, Any]:
    lexical = Path(discovery["path"])
    exists = lexical.exists()
    target_symlink = lexical.is_symlink()
    canonical = lexical.resolve(strict=False)
    unsafe_reasons = unsafe_target_reasons(canonical, discovery, protected_paths)
    tag = cache_tag_snapshot(lexical)
    stat = None
    if exists:
        try:
            raw = lexical.stat()
            stat = {
                "device": raw.st_dev,
                "inode": raw.st_ino,
                "mtime_ns": raw.st_mtime_ns,
            }
        except OSError:
            stat = None

    target_size = (
        du_bytes(lexical)
        if exists and not target_symlink and not unsafe_reasons
        else None
    )
    candidates: List[Dict[str, Any]] = []
    tag_valid = exists and lexical.is_dir() and not target_symlink and tag["exact"]
    active_reason = bool(active) or not process_scan_ok
    owner_protections = {
        owner: worktree_statuses.get(owner, {}).get("protected_reasons", [])
        for owner in discovery["worktrees"]
    }
    owner_protected = any(owner_protections.values())
    protection_reasons = sorted(
        {
            f"owning worktree {owner}: {reason}"
            for owner, reasons in owner_protections.items()
            for reason in reasons
        }
    ) + unsafe_reasons
    manually_reviewable = (
        tag_valid and not active_reason and not owner_protected and not unsafe_reasons
    )

    for profile_name in ("debug", "release"):
        profile = lexical / profile_name
        if not profile.is_dir() or profile.is_symlink():
            continue
        incremental = profile / "incremental"
        if incremental.is_dir() and not incremental.is_symlink():
            candidates.append(
                {
                    "path": str(incremental.resolve()),
                    "kind": f"{profile_name} incremental artifacts",
                    "bytes": du_bytes(incremental),
                    "automatic_eligibility": False,
                    "candidate_requires_manual_validation": manually_reviewable,
                    "reasons": []
                    if manually_reviewable
                    else (["invalid or missing target-root cache tag"] if not tag_valid else [])
                    + (["Rust/Cargo/link process is active"] if active else [])
                    + (["build-process inspection failed"] if not process_scan_ok else [])
                    + protection_reasons,
                }
            )

    candidates.append(
        {
            "path": str(canonical),
            "kind": "whole Cargo target root (high rebuild cost)",
            "bytes": target_size,
            "automatic_eligibility": False,
            "candidate_requires_manual_validation": manually_reviewable,
            "reasons": []
            if manually_reviewable
            else (["invalid or missing exact target-root cache tag"] if not tag_valid else [])
            + (["Rust/Cargo/link process is active"] if active else [])
            + (["build-process inspection failed"] if not process_scan_ok else [])
            + protection_reasons,
        }
    )

    return {
        **discovery,
        "canonical_path": str(canonical),
        "exists": exists,
        "target_path_is_symlink": target_symlink,
        "bytes": target_size,
        "identity": stat,
        "tag": tag,
        "active_processes": active,
        "classification": (
            "ACTIVE/LOCKED—DO NOT TOUCH"
            if active
            else "WORKTREE STATE—PROTECT"
            if owner_protected
            else "CARGO-SHAPED ARTIFACT—MANUAL VALIDATION REQUIRED"
            if manually_reviewable
            else "EVIDENCE/UNKNOWN—PROTECT"
        ),
        "candidates": candidates,
        "owner_protections": owner_protections,
        "filesystem": filesystem_snapshot(nearest_existing_parent(canonical)),
        "same_device_as_repo": bool(
            stat is not None and stat["device"] == repo_filesystem_device
        ),
        "unsafe_path_reasons": unsafe_reasons,
    }


def audit(repo_argument: Path) -> Dict[str, Any]:
    code, out, err = git(["rev-parse", "--show-toplevel"], repo_argument)
    if code != 0:
        raise RuntimeError(f"not a Git repository: {decode(err).strip()}")
    repo = Path(decode(out).strip()).resolve()
    worktrees = parse_worktrees(repo)
    target_discoveries = discover_target_roots(worktrees)
    worktree_target_roots = target_roots_by_worktree(target_discoveries)
    worktree_details = []
    for entry in worktrees:
        path = Path(entry["path"])
        status = status_snapshot(
            path,
            worktree_target_roots.get(str(normalized_path(path)), []),
        )
        metadata_reasons = []
        if entry.get("detached"):
            metadata_reasons.append("detached worktree")
        if entry.get("locked"):
            metadata_reasons.append("locked worktree")
        if entry.get("prunable"):
            metadata_reasons.append("prunable or missing worktree")
        if entry.get("bare"):
            metadata_reasons.append("bare worktree entry")
        status.setdefault("protected_reasons", []).extend(metadata_reasons)
        status["protected_reasons"] = sorted(set(status["protected_reasons"]))
        worktree_details.append({**entry, "status": status})

    process_before = running_rust_processes()
    active_before = process_before["processes"]
    worktree_statuses = {
        detail["path"]: detail["status"] for detail in worktree_details
    }
    repo_stat = repo.stat()
    protected_paths = registered_protected_paths(worktrees)
    targets = [
        target_snapshot(
            discovery,
            active_before,
            process_before["ok"],
            worktree_statuses,
            repo_stat.st_dev,
            protected_paths,
        )
        for discovery in target_discoveries
    ]
    process_after = running_rust_processes()
    active_after = process_after["processes"]
    active_by_pid = {
        process["pid"]: process for process in [*active_before, *active_after]
    }
    active = list(active_by_pid.values())
    process_snapshot_changed = active_before != active_after
    process_scan_ok = process_before["ok"] and process_after["ok"]
    if (active_after and not active_before) or not process_after["ok"]:
        for target in targets:
            target["classification"] = "ACTIVE/LOCKED—DO NOT TOUCH"
            target["active_processes"] = active_after
            for candidate in target["candidates"]:
                candidate["candidate_requires_manual_validation"] = False
                candidate["reasons"] = sorted(
                    set(
                        candidate["reasons"]
                        + (
                            ["build-process inspection failed during audit"]
                            if not process_after["ok"]
                            else ["build/link process appeared during audit"]
                        )
                    )
                )
    warnings = []
    if active:
        warnings.append(
            "Rust/Cargo/link processes were observed; do not clean any target from this snapshot."
        )
    if process_snapshot_changed:
        warnings.append("The build-process snapshot changed during the audit; rerun when idle.")
    if not process_scan_ok:
        warnings.append(
            "Build-process inspection failed; every cleanup candidate is protected."
        )
    if not targets:
        warnings.append("No existing or configured Cargo target roots were discovered.")
    if any(not target["tag"]["exact"] for target in targets if target["exists"]):
        warnings.append("At least one target lacks the exact non-symlink CACHEDIR.TAG signature.")
    if any(detail["status"].get("protected_reasons") for detail in worktree_details):
        warnings.append("One or more worktrees contain state that must be protected.")
    if any(
        "heuristic-config-parse" in confidence
        for target in targets
        for confidence in target["discovery_confidence"]
    ):
        warnings.append(
            "A target came from heuristic Cargo-config parsing; effective precedence is unresolved."
        )
    if any(not target["same_device_as_repo"] for target in targets if target["exists"]):
        warnings.append(
            "At least one target is on another device; its cleanup would not free repo-volume space."
        )

    return {
        "audit_version": AUDIT_VERSION,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "repo": str(repo),
        "filesystem": filesystem_snapshot(repo),
        "memory": memory_snapshot(),
        "running_processes": active,
        "process_scans": {"before": process_before, "after": process_after},
        "process_snapshot_changed": process_snapshot_changed,
        "worktrees": worktree_details,
        "target_roots": targets,
        "recommendations": [
            "Stop or coordinate duplicate active builds before considering cleanup.",
            "Protect every dirty, untracked, ignored-evidence, ahead, detached, or no-upstream worktree.",
            "Choose the smallest exact inactive Cargo artifact candidate that restores capacity.",
            "Revalidate path, device, inode, mtime, size, tag, and processes immediately before action.",
            "After authorized cleanup, rerun this audit and the identical failed verification gate.",
        ],
        "warnings": warnings,
    }


def print_human(report: Dict[str, Any]) -> None:
    fs = report["filesystem"]
    print(f"Cargo storage audit v{report['audit_version']}")
    print(f"Repository: {report['repo']}")
    print(
        "Filesystem: "
        f"{human_bytes(fs['bytes_free'])} free of {human_bytes(fs['bytes_total'])}; "
        f"{fs['inodes_free']:,} inodes free"
    )
    memory = report["memory"]
    print(
        "Memory: "
        f"{human_bytes(memory['bytes_available_estimate'])} available estimate of "
        f"{human_bytes(memory['bytes_total'])}; "
        f"swap {human_bytes(memory['swap_bytes_used'])} used of "
        f"{human_bytes(memory['swap_bytes_total'])}"
    )
    active = report["running_processes"]
    print(f"Active Cargo/Rust/game processes: {len(active)}")
    for proc in active:
        print(
            f"  PID {proc['pid']}: {proc['process']} "
            f"({human_bytes(proc.get('rss_bytes'))} RSS)"
        )

    print(f"Worktrees: {len(report['worktrees'])}")
    for item in report["worktrees"]:
        status = item["status"]
        branch = status.get("branch", {})
        counts = status.get("status_counts", {})
        print(
            f"  {item['path']} | {branch.get('name', item.get('branch', 'detached'))} | "
            f"HEAD {branch.get('head', item.get('HEAD', 'unknown'))} | "
            f"tracked={counts.get('tracked', '?')} untracked={counts.get('untracked', '?')} "
            f"ignored_target={counts.get('ignored_within_target', '?')} "
            f"ignored_other={counts.get('ignored_other', '?')} "
            f"ahead={branch.get('ahead', '?')}"
        )
        for reason in status.get("protected_reasons", []):
            print(f"    PROTECT: {reason}")

    print(f"Target roots: {len(report['target_roots'])}")
    for target in report["target_roots"]:
        print(
            f"  {target['canonical_path']} | {human_bytes(target['bytes'])} | "
            f"{target['classification']} | tag_exact={target['tag']['exact']} | "
            f"same_repo_device={target['same_device_as_repo']} | "
            f"free={human_bytes(target['filesystem']['bytes_free'])}"
        )
        for candidate in target["candidates"]:
            state = (
                "manual-validation-required"
                if candidate["candidate_requires_manual_validation"]
                else "protected"
            )
            print(
                f"    {state}: {candidate['kind']} | {human_bytes(candidate['bytes'])} | "
                f"{candidate['path']}"
            )
            for reason in candidate["reasons"]:
                print(f"      reason: {reason}")

    for warning in report["warnings"]:
        print(f"WARNING: {warning}")
    print("Read-only audit complete. No cleanup command was run or generated.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "repo",
        nargs="?",
        type=Path,
        default=Path.cwd(),
        help="Repository or worktree to audit (default: current directory)",
    )
    parser.add_argument("--json", action="store_true", help="Emit stable JSON instead of text")
    args = parser.parse_args()
    try:
        report = audit(args.repo)
    except (OSError, RuntimeError) as error:
        print(f"audit failed: {error}", file=sys.stderr)
        return 2
    if args.json:
        json.dump(report, sys.stdout, indent=2, sort_keys=True, ensure_ascii=False)
        sys.stdout.write("\n")
    else:
        print_human(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
