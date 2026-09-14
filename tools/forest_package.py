#!/usr/bin/env python3
"""Reproduce or verify the complete expedition through the supported world CLI.

Compile/verify use an explicitly built compiler. The launch-only ensure command
builds a missing compiler/package once in an isolated target. No command reads
private package files or changes the legacy Forest source.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import errno
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from forest_expedition import document, site_document
from forest_finish import compose, final_source
from forest_verify import verify
from forest_world import ROOT, Raw, ron
import world as world_tool

CONTENT = ROOT / "assets/config/v4/forest-massif/expedition"
DEFAULT_OUTPUT = CONTENT / "compiled"
DEFAULT_TARGET = ROOT / "target/v4-authoring"


@contextmanager
def preparation_lock(output: Path):
    """Serialize launch preparations, including the ready-package recheck.

    Keep the lock file inside the ignored package workspace and never unlink it:
    waiting processes must keep referring to the same OS lock after publication.
    Process exit releases the lock even when a compiler fails or is interrupted.
    """
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".preparation.lock").open("a+b") as lock:
        if os.name == "nt":
            import msvcrt
            if os.fstat(lock.fileno()).st_size == 0:
                lock.write(b"\0")
                lock.flush()
            while True:
                try:
                    lock.seek(0)
                    msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError as error:
                    if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                        raise
                    time.sleep(.1)
            try:
                yield
            finally:
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def atomic_text(path: Path, value: str) -> None:
    """Publish the complete companion without exposing a truncated RON file."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                         delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def package_ready(output: Path, generation: dict) -> bool:
    """Cheap launch preflight; the runtime still validates all authoritative bytes.

    Require both public compiler and content verification receipts from the
    reviewed generation, plus the runtime pointer and exact companion. Missing,
    stale or interrupted publication rebuilds; no full survey runs on a ready map.
    """
    try:
        if not all((output / name).is_file() for name in
                   ("current.ron", "arena-sites.ron", "compile-receipt.json", "content-verification.json")):
            return False
        if any((output / name).stat().st_size == 0 for name in ("current.ron", "arena-sites.ron")):
            return False
        compiled = json.loads((output / "compile-receipt.json").read_text())
        verified = json.loads((output / "content-verification.json").read_text())
        fingerprint = generation["package_fingerprint"]
        return (compiled.get("strict") is True
                and compiled.get("package_fingerprint") == fingerprint
                and verified.get("version") == 1
                and verified.get("world_id") == generation["world_id"]
                and verified.get("manifest_fingerprint") == int(fingerprint, 16))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        return False


def ensure_package(target: Path, output: Path, scratch: Path) -> bool:
    """Prepare the implicit expedition only when its reviewed package is missing.

    Child Cargo always gets a separate authoring target, including cargo metadata;
    it must never inherit the running app's build target or lock.
    """
    output = output.resolve()
    with preparation_lock(output):
        return _ensure_package(target, output, scratch)


def _ensure_package(target: Path, output: Path, scratch: Path) -> bool:
    # A preceding launcher may have completed while this process waited.
    generation = json.loads((CONTENT / "generation.json").read_text())
    if package_ready(output, generation):
        return False
    target = target.resolve()
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2")
    print("Preparing the reviewed Forest Expedition package for the first launch…", flush=True)
    try:
        world_tool.checked_binary(target)
    except (OSError, ValueError):
        subprocess.run([sys.executable, str(ROOT / "tools/world.py"), "--target-dir", str(target), "build"],
                       cwd=ROOT, env=env, check=True)
    scratch.mkdir(parents=True, exist_ok=True)
    # Retain each failed/successful preparation's evidence without sharing its
    # intermediate files with another output or an explicit authoring command.
    working = Path(tempfile.mkdtemp(prefix="prepare-", dir=scratch.resolve()))
    subprocess.run([sys.executable, str(ROOT / "tools/forest_package.py"), "compile",
                    "--target-dir", str(target), "--output", str(output), "--scratch", str(working)],
                   cwd=ROOT, env=env, check=True)
    if not package_ready(output, generation):
        raise ValueError("Forest package preparation did not publish the reviewed expedition; launch stopped")
    return True


def run_world(target, *args):
    subprocess.run([sys.executable, str(ROOT / "tools/world.py"), "--target-dir", str(target), *map(str,args)],
                   cwd=ROOT, check=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command",choices=("compile","verify","ensure"))
    parser.add_argument("--target-dir",type=Path,default=DEFAULT_TARGET)
    parser.add_argument("--output",type=Path,default=DEFAULT_OUTPUT)
    parser.add_argument("--scratch",type=Path,default=ROOT/".context/expedition-reproduction")
    args=parser.parse_args()
    if args.command == "ensure":
        ensure_package(args.target_dir, args.output, args.scratch)
        return
    args.scratch.mkdir(parents=True,exist_ok=True)
    generation=json.loads((CONTENT/"generation.json").read_text())
    terrain,_=document(raw=Raw,ron=ron)
    if terrain != (CONTENT/"terrain.ron").read_text():
        raise ValueError("terrain generator differs from reviewed source")
    run_world(args.target_dir,"compile","--source",CONTENT/"terrain.ron","--output",args.scratch/"terrain-compiled")
    run_world(args.target_dir,"survey","--package",args.scratch/"terrain-compiled","--output",args.scratch/"terrain-survey.json")
    survey=json.loads((args.scratch/"terrain-survey.json").read_text())
    value,metadata,assets,outputs,placement,_=compose(survey)
    revision=generation["source_revision"]
    # Keep exported provenance stable after an identical-artwork cherry-pick.
    # Verification must use a commit carried by the published candidate history.
    art_revision=generation.get("art_verification_revision", revision)
    for path,expected in outputs.items():
        committed=subprocess.run(["git","show",f"{art_revision}:{path}"],cwd=ROOT,check=True,capture_output=True,text=True).stdout
        if committed != expected or (ROOT/path).read_text() != expected:
            raise ValueError(f"current or committed exact artwork differs: {path}")
    source=final_source(value,placement,assets,revision)
    if source != (CONTENT/"world.ron").read_text():
        raise ValueError("terrain-aware placements no longer reproduce reviewed source")
    if metadata != json.loads((CONTENT/"authoring.json").read_text()):
        # Tuples are normalized by JSON without changing authored coordinates.
        if json.loads(json.dumps(metadata)) != json.loads((CONTENT/"authoring.json").read_text()):
            raise ValueError("site metadata differs from reviewed authoring")
    if args.command=="compile":
        run_world(args.target_dir,"compile","--source",CONTENT/"world.ron","--output",args.output)
    run_world(args.target_dir,"survey","--package",args.output,"--output",args.scratch/"final-survey.json")
    survey=json.loads((args.scratch/"final-survey.json").read_text())
    evidence=verify(survey,metadata,placement)
    if f'{survey["manifest_fingerprint"]:016x}' != generation["package_fingerprint"]:
        raise ValueError("reproduced package fingerprint differs from reviewed source")
    sidecar=site_document(metadata,world_id=survey["world_id"],manifest_fingerprint=survey["manifest_fingerprint"],raw=Raw,ron=ron)
    if args.command=="compile":
        atomic_text(args.output/"arena-sites.ron", sidecar)
        # Publish acceptance last, after the entire companion is visible.
        world_tool.atomic_json(args.output/"content-verification.json", evidence)
    elif (args.output/"arena-sites.ron").read_text() != sidecar:
        raise ValueError("package companion differs from verified authoring")
    (args.scratch/"verification.json").write_text(json.dumps(evidence,indent=2)+"\n")
    print(json.dumps(evidence))


if __name__=="__main__":
    main()
