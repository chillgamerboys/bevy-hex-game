#!/usr/bin/env python3
"""Explicit native launch and windowless, provenance-bearing arena captures."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import time
from datetime import datetime, timezone


ROOT = Path(__file__).resolve().parents[1]
LOCAL_TARGET = Path(
    "/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/"
    "work/cargo-target-explore"
)


def default_target() -> Path:
    """Honor the operator's cache; reuse the retained local cache only if present."""
    configured = os.environ.get("CARGO_TARGET_DIR")
    if configured:
        path = Path(configured).expanduser()
        return path.resolve() if path.is_absolute() else (ROOT / path).resolve()
    return LOCAL_TARGET if LOCAL_TARGET.is_dir() else ROOT / "target"


DEFAULT_TARGET = default_target()
VIEWS = (
    "overview", "first", "third", "rear", "shield", "fireball", "blast", "tuning",
    "shield-compact", "shield-large", "fireball-compact", "fireball-large",
    "blast-compact", "blast-large",
    "shield-first", "shield-third", "fireball-first", "fireball-third",
    "blast-first", "blast-third", "shield-preview-first", "shield-preview-third", "start",
)
MATRIX = "arena-v5-release-casting"
MENU_VIEWS = ("start", "tuning", "first", "third", "overview", "rear", "terminal-win", "terminal-defeat")
BOT_VIEWS = ("bot-combat-first", "bot-combat-third")
CHARGE_VIEWS = (
    "start", "tuning", "first", "third",
    "shield-charge-partial-first", "shield-charge-partial-third",
    "shield-charge-full-first", "shield-charge-full-third",
    "fireball-charge-partial-first", "fireball-charge-partial-third",
    "fireball-charge-full-first", "fireball-charge-full-third",
    "blast-armed-first", "blast-armed-third",
    "shield-partial-preview-first", "shield-partial-preview-third",
)
# Explicit recipes preserve the legacy two-actor regression matrices.
MAPS = ("duel", "fort", "seven-regions")
ENCOUNTERS = ("dragon", "goblins", "shaman-party", "shadow", "golem", "goblin", "wisp", "wisps-2", "wisps-4", "wisps-8", "wisps-12", "worm")
PRESET_MEMBERS = {"shadow": ["Shadow"], "dragon": ["Dragon"], "goblins": ["Goblin"] * 10,
                  "shaman-party": ["Shaman", *(["Goblin"] * 5)], "golem": ["Golem"],
                  "worm": ["Worm"], "goblin": ["Goblin"], "wisp": ["Wisp"], **{f"wisps-{n}": ["Wisp"] * n for n in (2, 4, 8, 12)}}
PLAYER_OVERRIDES = {"worm": "Worm", "golem": "Golem", "goblin": "Goblin", "wisp": "Wisp", "wisps-2": "Wisps2", "wisps-4": "Wisps4", "wisps-8": "Wisps8", "wisps-12": "Wisps12"}
MAP_LABELS = {"duel": "Duel", "fort": "Fort", "seven-regions": "Seven Regions"}
ENCOUNTER_LABELS = {"dragon": "Dragon", "goblins": "Goblins", "shaman-party": "Shaman party", "shadow": "Shadow", "golem": "Golem"}
SEEDS = {"duel": None, "fort": 640367719, "seven-regions": 703700113}
ENCOUNTER_VIEWS = (
    ("fort-dragon-start", "start", "fort", "dragon", None),
    ("fort-dragon-overview", "overview", "fort", "dragon", None),
    ("fort-dragon-rear", "rear", "fort", "dragon", None),
    ("fort-dragon-first", "encounter-first", "fort", "dragon", None),
    ("fort-dragon-third", "encounter-third", "fort", "dragon", None),
    ("dragon-body-rear", "encounter-body-rear", "fort", "dragon", None),
    ("dragon-windup", "encounter-windup", "fort", "dragon", None),
    ("dragon-breath", "encounter-breath", "fort", "dragon", None),
    ("dragon-barrier", "encounter-barrier", "fort", "dragon", None),
    ("dragon-barrier-rear", "encounter-barrier-rear", "fort", "dragon", None),
    ("fort-goblins-third", "encounter-third", "fort", "goblins", None),
    ("goblin-body-rear", "encounter-body-rear", "fort", "goblins", None),
    ("goblin-swipe", "encounter-swipe", "fort", "goblins", None),
    ("fort-shaman-third", "encounter-third", "fort", "shaman-party", None),
    ("shaman-body-rear", "encounter-body-rear", "fort", "shaman-party", None),
    ("shaman-fireball", "encounter-fireball", "fort", "shaman-party", None),
    ("shaman-aura", "encounter-aura", "fort", "shaman-party", None),
    ("shaman-aura-rear", "encounter-aura-rear", "fort", "shaman-party", None),
    ("fort-shadow-first", "encounter-first", "fort", "shadow", None),
    ("seven-start", "start", "seven-regions", "dragon", None),
    ("seven-overview", "overview", "seven-regions", "dragon", None),
    ("seven-rear", "rear", "seven-regions", "dragon", None),
    ("seven-first", "first", "seven-regions", "dragon", None),
    ("seven-third", "third", "seven-regions", "dragon", None),
    ("seven-mountains", "encounter-landmark", "seven-regions", "dragon", "mountains_high_pass"),
    ("seven-fort", "encounter-landmark", "seven-regions", "dragon", "fort_fort_courtyard"),
    ("seven-caves", "encounter-landmark", "seven-regions", "dragon", "caves_cave_entrance"),
)
PERFORMANCE_VIEWS = (
    ("fort-dragon-stress", "encounter-stress", "fort", "dragon", None),
    ("fort-goblins-stress", "encounter-stress", "fort", "goblins", None),
    ("fort-shaman-stress", "encounter-stress", "fort", "shaman-party", None),
    ("fort-shadow-stress", "encounter-stress", "fort", "shadow", None),
    ("seven-all-enemies-stress", "encounter-stress", "seven-regions", "dragon", None),
)
OBSERVER_VIEWS = (
    ("fort-observer-start", "observer-start", "fort", "shadow", None),
    ("fort-observer-orbit", "observer-orbit", "fort", "shadow", None),
    ("fort-observer-rear", "observer-orbit-rear", "fort", "shadow", None),
    ("fort-observer-close", "observer-close", "fort", "shadow", None),
    ("fort-observer-close-rear", "observer-close-rear", "fort", "shadow", None),
    ("fort-observer-free", "observer-free", "fort", "shadow", None),
    ("fort-observer-paused", "observer-paused", "fort", "shadow", None),
    ("fort-observer-result", "observer-result", "fort", "shadow", None),
    ("duel-observer-start", "observer-start", "duel", "shadow", None),
    ("duel-observer-orbit", "observer-orbit", "duel", "shadow", None),
    ("duel-observer-close", "observer-close", "duel", "shadow", None),
    ("duel-observer-close-rear", "observer-close-rear", "duel", "shadow", None),
    ("duel-observer-free", "observer-free", "duel", "shadow", None),
    ("duel-observer-result", "observer-result", "duel", "shadow", None),
)
# Fort player override plus neutral Duel observer laser phases; no actor injection.
# Duel tuples keep the legacy world recipe; the actual teams are explicit below.
GOLEM_OBSERVER_PRESETS = ("golem", "dragon")
GOLEM_VIEWS = (
    ("fort-golem-start", "start", "fort", "golem", None),
    ("fort-golem-first", "encounter-first", "fort", "golem", None),
    ("fort-golem-third", "encounter-third", "fort", "golem", None),
    ("fort-golem-body-rear", "encounter-body-rear", "fort", "golem", None),
    ("fort-golem-slam-windup", "encounter-golem-slam-windup", "fort", "golem", None),
    ("fort-golem-slam", "encounter-golem-slam", "fort", "golem", None),
    ("fort-golem-swipe-windup", "encounter-golem-swipe-windup", "fort", "golem", None),
    ("fort-golem-swipe", "encounter-golem-swipe", "fort", "golem", None),
    ("duel-golem-charge", "encounter-golem-charge", "duel", "shadow", None),
    ("duel-golem-charge-late", "encounter-golem-charge-late", "duel", "shadow", None),
    ("duel-golem-beam", "encounter-golem-beam", "duel", "shadow", None),
    ("duel-golem-beam-rear", "encounter-golem-beam-rear", "duel", "shadow", None),
    ("duel-golem-observer-close", "observer-close", "duel", "shadow", None),
    ("duel-golem-observer-close-rear", "observer-close-rear", "duel", "shadow", None),
)
WISP_VIEWS = (
    ("fort-wisps-start", "start", "fort", "wisps-4", None),
    ("fort-wisps-first", "encounter-first", "fort", "wisps-4", None),
    ("fort-wisps-third", "encounter-third", "fort", "wisps-4", None),
    ("duel-wisp-body", "encounter-wisp-body", "duel", "shadow", None),
    ("duel-wisp-body-rear", "encounter-wisp-body-rear", "duel", "shadow", None),
    ("duel-wisp-dark", "encounter-wisp-dark", "duel", "shadow", None),
    ("duel-wisp-dark-rear", "encounter-wisp-dark-rear", "duel", "shadow", None),
    ("duel-wisp-windup", "encounter-wisp-windup", "duel", "shadow", None),
    ("duel-wisp-ember", "encounter-wisp-ember", "duel", "shadow", None),
    ("duel-wisp-ember-rear", "encounter-wisp-ember-rear", "duel", "shadow", None),
    ("fort-wisps-observer-start", "observer-start", "fort", "shadow", None),
    ("duel-wisps-layers", "encounter-wisp-layers", "duel", "shadow", None),
)
WISP_OBSERVER_RECIPES = {name: ("wisp", "goblin") for name, _, arena_map, _, _ in WISP_VIEWS if arena_map == "duel"}
WISP_OBSERVER_RECIPES.update({"fort-wisps-observer-start": ("wisps-12", "shaman-party"),
                               "duel-wisps-layers": ("wisps-12", "wisps-12")})
WORM_VIEWS = (
    ('fort-worm-start', 'start', 'fort', 'worm', None),
    ('fort-worm-overview', 'overview', 'fort', 'worm', None),
    ('fort-worm-first', 'encounter-first', 'fort', 'worm', None),
    ('fort-worm-third', 'encounter-third', 'fort', 'worm', None),
    ('duel-worm-body', 'encounter-worm-body', 'duel', 'shadow', None),
    ('duel-worm-body-rear', 'encounter-worm-body-rear', 'duel', 'shadow', None),
    ('duel-worm-buried', 'encounter-worm-buried', 'duel', 'shadow', None),
    ('duel-worm-emerging', 'encounter-worm-emerging', 'duel', 'shadow', None),
    ('duel-worm-windup', 'encounter-worm-windup', 'duel', 'shadow', None),
    ('duel-worm-boulder', 'encounter-worm-boulder', 'duel', 'shadow', None),
    ('duel-worm-boulder-rear', 'encounter-worm-boulder-rear', 'duel', 'shadow', None),
    ('duel-worm-converted-earth', 'encounter-worm-converted-earth', 'duel', 'shadow', None),
    ('fort-worm-reset', 'encounter-worm-reset', 'fort', 'worm', None),
)
WORM_OBSERVER_PRESETS = ("worm", "goblins")
WISP_PERFORMANCE_VIEWS = tuple(
    (f"{arena_map}-wisps-stress", "observer-wisp-stress", arena_map, "shadow", None)
    for arena_map in ("duel", "fort")
)
OBSERVER_PERFORMANCE_VIEWS = tuple(
    (f"{arena_map}-observer-performance", "observer-performance", arena_map, "shadow", None)
    for arena_map in ("fort", "duel")
)


def battle_environment(args: argparse.Namespace, arena_map: str, *, matrix: bool = False, result: bool = False) -> dict[str, str]:
    if args.encounter in PLAYER_OVERRIDES and arena_map not in ("fort", "duel"):
        raise RuntimeError("Creature player overrides require Fort or Duel.")
    observing = args.spectator or matrix
    if not observing:
        if any(value is not None for value in (args.team_a, args.team_b, args.seed, args.tick_limit)):
            raise RuntimeError("--team-a, --team-b, --seed and --tick-limit require --spectator.")
        return {}
    if getattr(args, "encounter", None) is not None:
        raise RuntimeError("Spectator battles use --team-a and --team-b; --encounter is a player option.")
    if arena_map == "seven-regions":
        raise RuntimeError("Spectator battles support only Fort and Duel.")
    seed = args.seed if args.seed is not None else 1
    limit = args.tick_limit if args.tick_limit is not None else (720 if result else 14400)
    if not 0 <= seed <= 2**64 - 1 or not 1 <= limit <= 2**64 - 1:
        raise RuntimeError("Seed must fit unsigned 64 bits and tick limit must be positive and fit unsigned 64 bits.")
    return {"HEX_ARENA_CONTROL": "spectator", "HEX_ARENA_TEAM_A": args.team_a or ("shadow" if matrix and arena_map == "duel" else "goblins"),
            "HEX_ARENA_TEAM_B": args.team_b or ("dragon" if matrix and arena_map == "duel" else "shaman-party"), "HEX_ARENA_BATTLE_SEED": str(seed),
            "HEX_ARENA_BATTLE_TICK_LIMIT": str(limit)}


CANVAS = [1600, 900]
CARGO_ARGS = ("run", "-p", "hex_game", "--features", "dev,arena-prototype", "--", "--arena")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def git(*args: str) -> bytes:
    return subprocess.check_output(("git", *args), cwd=ROOT, stderr=subprocess.PIPE)


def source_state() -> tuple[dict, bytes, bytes]:
    """Fingerprint staged, unstaged, and untracked source without printing contents."""
    staged = git("diff", "--cached", "--binary", "HEAD", "--")
    unstaged = git("diff", "--binary", "--")
    status = git("status", "--porcelain=v1", "-z", "--untracked-files=all")
    untracked = {}
    for encoded in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0"):
        if encoded:
            name = os.fsdecode(encoded)
            path = ROOT / name
            data = os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes()
            untracked[name] = digest(data)
    state = {
        "head": git("rev-parse", "HEAD").decode().strip(),
        "status_porcelain_z": os.fsdecode(status),
        "staged_diff_sha256": digest(staged),
        "unstaged_diff_sha256": digest(unstaged),
        "untracked_sha256": untracked,
        "dirty": bool(status),
    }
    state["state_sha256"] = digest(json.dumps(state, sort_keys=True).encode())
    return state, staged, unstaged


def environment(target: Path) -> tuple[dict[str, str], list[str]]:
    """Discard inherited game capabilities; only this invocation may opt them in."""
    env = dict(os.environ)
    removed = sorted(key for key in env if key.startswith("HEX_"))
    for key in removed:
        del env[key]
    # Let .cargo/config.toml supply the checkout's asset root, even from Finder.
    env.pop("BEVY_ASSET_ROOT", None)
    env.update(CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1")
    cargo = shutil.which("cargo", path=env.get("PATH"))
    if cargo is None:
        fallback = Path.home() / ".cargo" / "bin" / "cargo"
        if not fallback.is_file():
            raise RuntimeError("Cargo is unavailable. Install the repository's pinned Rust toolchain.")
        env["PATH"] = str(fallback.parent) + os.pathsep + env.get("PATH", "")
    return env, removed


def stop_process(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def run_cargo(env: dict[str, str], log_path: Path | None, timeout: float | None) -> int:
    """Stream native output, retain capture logs, and stop on missing asset errors."""
    command = ("cargo", *CARGO_ARGS)
    log = log_path.open("wb") if log_path else None
    process = None
    started = time.monotonic()
    tail = b""
    try:
        process = subprocess.Popen(
            command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
        assert process.stdout is not None
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                if timeout is not None and time.monotonic() - started > timeout:
                    raise RuntimeError(f"Capture exceeded {timeout:g} seconds; native fallback is forbidden.")
                for key, _ in selector.select(timeout=0.25):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    if log:
                        log.write(chunk)
                        log.flush()
                    else:
                        sys.stdout.buffer.write(chunk)
                        sys.stdout.buffer.flush()
                    observed = (tail + chunk).lower()
                    if b"path not found" in observed:
                        raise RuntimeError("Asset path not found; Cargo launch failed. See process output.")
                    if b"cannot render authored object" in observed or b"cannot render invalid authored object" in observed:
                        raise RuntimeError("Authored object rendering failed. See retained process output.")
                    if any(message in observed for message in (
                        b"arena crystal presentation:", b"arena liquid presentation:",
                        b"arena feature presentation:",
                    )):
                        raise RuntimeError("Arena world presentation failed. See retained process output.")
                    tail = chunk[-96:]
        return process.wait()
    finally:
        if process is not None:
            stop_process(process)
            if process.stdout:
                process.stdout.close()
        if log:
            log.close()


def png_info(path: Path) -> dict:
    data = path.read_bytes()
    if len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise RuntimeError(f"Missing or invalid PNG: {path}")
    width, height = struct.unpack(">II", data[16:24])
    if not width or not height:
        raise RuntimeError(f"Empty PNG dimensions: {path}")
    return {"file": path.name, "sha256": digest(data), "bytes": len(data),
            "physical_pixels": [width, height]}


def validate_capture_setup(state: dict, arena_map: str, encounter: str, env: dict[str, str]) -> None:
    """Check accepted setup and actual bodies for selectable player parties."""
    expected_selection = {"map": MAP_LABELS[arena_map],
                          "encounter": ENCOUNTER_LABELS["dragon" if encounter in PLAYER_OVERRIDES else encounter]}
    if state.get("selection") != expected_selection:
        raise RuntimeError("Capture published a different world selection than requested.")
    observing = env.get("HEX_ARENA_CONTROL") == "spectator"
    rosters = [{"team": team, "parties": [PRESET_MEMBERS[env[key]]]} for team, key in
               ((1, "HEX_ARENA_TEAM_A"), (2, "HEX_ARENA_TEAM_B"))] if observing else [
                   {"team": 1, "parties": [["Shadow"]]}, {"team": 2, "parties": [["Dragon"]]}]
    expected = {"control": "Spectator" if observing else "Player", "rosters": rosters,
                "seed": int(env.get("HEX_ARENA_BATTLE_SEED", "1")),
                "tick_limit": int(env.get("HEX_ARENA_BATTLE_TICK_LIMIT", "14400")),
                "player_recipe": None if observing else PLAYER_OVERRIDES.get(encounter,
                    {"dragon": "Dragon", "goblins": "Goblins", "shaman-party": "ShamanParty"}.get(encounter)
                    if arena_map == "duel" else None)}
    if state.get("battle_setup") != expected:
        raise RuntimeError("Capture accepted a different control, roster, seed, tick limit or player recipe.")
    if observing:
        expected_members = Counter((roster["team"], species) for roster in rosters for party in roster["parties"] for species in party)
        actual_members = Counter((actor.get("team"), actor.get("species")) for actor in state.get("actors", []))
        if actual_members != expected_members:
            raise RuntimeError("Capture actual bodies differ from the accepted spectator teams.")
    elif (encounter in PLAYER_OVERRIDES or arena_map == "duel") and Counter(actor.get("species") for actor in state.get("actors", [])) != Counter(["Human", *PRESET_MEMBERS[encounter]]):
        raise RuntimeError("Player party selection published different actual creature bodies.")


def validate_golem_state(state: dict, view: str) -> None:
    """Typed geometry/action evidence; visual correctness still requires raw-image review."""
    def vector(value: object, size: int = 3) -> bool:
        return isinstance(value, list) and len(value) == size and all(
            type(v) in (int, float) and math.isfinite(v) for v in value)

    def near(actual: object, expected: list[float]) -> bool:
        return vector(actual, len(expected)) and all(abs(a - b) < 0.001 for a, b in zip(actual, expected))

    golems = [actor for actor in state.get("actors", []) if actor.get("species") == "Golem"]
    phase = view.removesuffix("-rear")
    needs_golem = phase.startswith("encounter-golem-") or state.get("battle_setup", {}).get("player_recipe") == "Golem"
    if needs_golem and not golems:
        raise RuntimeError("Golem capture did not initialize its actual Golem body.")
    expected_offsets = [[0, 0, 0], [math.sqrt(3), 0, 0], [math.sqrt(3) / 2, 0, 1.5],
                        [-math.sqrt(3) / 2, 0, 1.5], [-math.sqrt(3), 0, 0],
                        [-math.sqrt(3) / 2, 0, -1.5], [math.sqrt(3) / 2, 0, -1.5]]
    for actor in golems:
        prisms = actor.get("body_hex_prisms", [])
        if len(prisms) != 7 or not all(type(p.get("height")) in (int, float) and abs(p["height"] - 2) < 0.001 for p in prisms):
            raise RuntimeError("Golem body is not seven full-height physical prisms.")
        if not all(sum(near(p.get("offset"), offset) for p in prisms) == 1 for offset in expected_offsets):
            raise RuntimeError("Golem physical prism footprint differs from the native seven-hex union.")
        if not near(actor.get("body_dimensions"), [3 * math.sqrt(3), 2, 5]) or not near(actor.get("body_rotation"), [0, 0, 0, 1]):
            raise RuntimeError("Golem body bounds/yaw differ from its fixed world-oriented geometry.")
        if not vector(actor.get("idle_mouth")):
            raise RuntimeError("Golem receipt lacks its authoritative mouth.")
        beam = actor.get("beam")
        if beam is not None:
            if not isinstance(beam, dict) or not all(vector(beam.get(key)) for key in ("origin", "direction", "end")) or type(beam.get("tracking")) is not bool:
                raise RuntimeError("Golem beam snapshot contains invalid vectors/tracking state.")
            radius = beam.get("radius")
            direction, delta = beam["direction"], [b - a for a, b in zip(beam["origin"], beam["end"])]
            length = sum(a * b for a, b in zip(delta, direction))
            if type(radius) not in (int, float) or not math.isfinite(radius) or radius <= 0 or abs(sum(d*d for d in direction) - 1) > 0.001 or length < 0 or any(abs(a - length*b) > 0.01 for a, b in zip(delta, direction)):
                raise RuntimeError("Golem beam is not a finite forward ray with its physical radius.")
    if golems and state.get("golem_render_prisms") != 7 * len(golems):
        raise RuntimeError("Golem physical columns have not all produced render meshes.")
    if not phase.startswith("encounter-golem-"):
        return
    reached = state.get("phase_reached_frame")
    if type(reached) is not int or state.get("frame", 0) < reached + 4:
        raise RuntimeError("Golem phase capture lacks four completed frozen render frames.")
    for actor in golems:
        if actor.get("hp", 0) <= 0:
            continue
        attack, beam = actor.get("attack") or {}, actor.get("beam") or {}
        kind, stage, progress = attack.get("kind"), attack.get("phase"), attack.get("progress", -1)
        laser = kind == "GolemLaser"
        valid = {
            "encounter-golem-charge": laser and stage == "Windup" and 0.25 <= progress <= 0.55 and bool(beam),
            "encounter-golem-charge-late": laser and stage == "Windup" and progress >= 0.75 and bool(beam),
            "encounter-golem-beam": laser and stage == "Active" and bool(beam),
            "encounter-golem-slam-windup": kind == "GolemSlam" and stage == "Windup" and progress >= 0.25,
            "encounter-golem-swipe-windup": kind == "GolemSwipe" and stage == "Windup" and progress >= 0.25,
            "encounter-golem-swipe": kind == "GolemSwipe" and stage == "Active" and progress >= 0.1,
            "encounter-golem-slam": kind == "GolemSlam" and stage == "Active" and any(
                e.get("spell") == "AreaBlast" and 0 < e.get("age", -1) <= 0.10
                and abs(e.get("radius", -1) - attack.get("range", -10)) < 0.01
                and vector(e.get("center")) and vector(attack.get("origin"))
                and math.dist(e["center"], attack["origin"]) < 0.5 for e in state.get("effects", [])),
        }.get(phase, False)
        if valid:
            return
    raise RuntimeError("Golem capture lacks the requested natural ability phase.")


def validate_wisp_state(state: dict, view: str) -> None:
    def vector(value: object) -> bool:
        return isinstance(value, list) and len(value) == 3 and all(type(v) in (int, float) and math.isfinite(v) for v in value)

    wisps = [a for a in state.get("actors", []) if a.get("species") == "Wisp"]
    phase = view.removesuffix("-rear")
    if phase.startswith("encounter-wisp-") and not wisps:
        raise RuntimeError("Wisp view did not publish its actual Wisp body.")
    for actor in wisps:
        prisms = actor.get("body_hex_prisms", [])
        dimensions = actor.get("body_dimensions")
        height = prisms[0].get("height") if len(prisms) == 1 else None
        if len(prisms) != 1 or not vector(prisms[0].get("offset")) or prisms[0]["offset"] != [0, 0, 0] or type(height) not in (int, float) or not math.isfinite(height) or abs(height - .4) > .001:
            raise RuntimeError("Wisp must be exactly one native hex prism one level tall.")
        if not vector(dimensions) or any(abs(a-b) > .001 for a, b in zip(dimensions, [math.sqrt(3), .4, 2])) or actor.get("body_rotation") != [0, 0, 0, 1]:
            raise RuntimeError("Wisp physical bounds or fixed yaw disagree with its native body.")
        if not vector(actor.get("idle_mouth")) or not vector(actor.get("feet")) or math.dist(actor["idle_mouth"], [actor["feet"][0], actor["feet"][1] + .2, actor["feet"][2]]) > .001:
            raise RuntimeError("Wisp receipt lacks its physical glow core.")
    if wisps and state.get("wisp_render_prisms") != len(wisps):
        raise RuntimeError("Wisp physical bodies have not all produced render meshes.")
    embers = [p for p in state.get("projectiles", []) if p.get("appearance") == "Ember"]
    for projectile in embers:
        radius = projectile.get("collision_radius")
        if projectile.get("source_ability") != "WispEmber" or type(radius) not in (int, float) or not 0 < radius <= .25:
            raise RuntimeError("Ember lacks its frozen creature appearance and physical radius.")
        if not all(vector(projectile.get(key)) for key in ("position", "previous_position", "velocity")):
            raise RuntimeError("Ember has invalid physical pose metadata.")
    if not phase.startswith("encounter-wisp-"):
        return
    reached = state.get("phase_reached_frame")
    if type(reached) is not int or state.get("frame", 0) < reached + 4:
        raise RuntimeError("Wisp phase lacks four frozen render frames.")
    alive = [a for a in wisps if a.get("hp", 0) > 0 and a.get("flying") is True]
    if not alive:
        raise RuntimeError("Wisp capture requires an actual living flying subject.")
    if phase == "encounter-wisp-dark":
        light = state.get("lighting", {})
        if light.get("fixture") != "dim-comparison" or light.get("ambient_brightness") != 6 or light.get("directional_illuminance") != [0]:
            raise RuntimeError("Dark Wisp capture did not use the explicit dim-light comparison.")
    elif phase == "encounter-wisp-windup":
        if state.get("wisp_windup_segments", 0) < 6:
            raise RuntimeError("Wisp windup has not produced its local warning ring.")
        if not any((a.get("attack") or {}).get("kind") == "WispEmber" and (a.get("attack") or {}).get("phase") == "Windup" and .3 <= (a.get("attack") or {}).get("progress", -1) <= .7 for a in alive):
            raise RuntimeError("Wisp capture lacks its natural Ember windup.")
    elif phase == "encounter-wisp-ember":
        if not any(p.get("owner") == a.get("id") and p.get("age", 0) > 0 and math.dist(p["position"], a["idle_mouth"]) > 1.2 for a in alive for p in embers):
            raise RuntimeError("Wisp capture lacks a genuinely released Ember outside its owner body.")
    elif phase == "encounter-wisp-layers":
        if len(state.get("actors", [])) != 24 or len(alive) != 24:
            raise RuntimeError("Layered capture requires all 24 actual living Wisp bodies.")
        teams = {actor.get("team") for actor in alive}
        if len(teams) != 2 or any({a.get("flight_layer") for a in alive if a.get("team") == team} != {0, 1} for team in teams):
            raise RuntimeError("Layered capture requires both published flight layers on each team.")


def validate_worm_state(state: dict, view: str) -> None:
    """Current public shapes, frozen shots and acknowledged edits; never a visual verdict."""
    def number(value):
        return type(value) in (int, float) and math.isfinite(value)

    def vector(value, size=3):
        return isinstance(value, list) and len(value) == size and all(number(v) for v in value)

    def near(value, expected):
        return vector(value, len(expected)) and all(abs(a-b) < .001 for a, b in zip(value, expected))

    def require(condition, message):
        if not condition:
            raise RuntimeError("Worm capture: " + message)

    worms = [actor for actor in state.get("actors", []) if actor.get("species") == "Worm"]
    phase = view.removesuffix("-rear")
    requested = phase.startswith("encounter-worm-") or state.get("battle_setup", {}).get("player_recipe") == "Worm"
    require(not requested or worms, "no actual Worm admitted")
    expected_parts = {}
    for actor in worms:
        parts = actor.get("body_hex_prisms", [])
        require(len(parts) in (4, 6), "body must contain four or six native prisms")
        require(vector(actor.get("feet")), "invalid physical feet")
        for index, part in enumerate(parts):
            require(vector(part.get("offset")) and number(part.get("height")) and abs(part["height"]-.4) < .001,
                    "invalid one-level physical segment")
            expected_parts[(actor.get("id"), index)] = ([part["offset"][0], part["offset"][1]+.2, part["offset"][2]], [1, .4, 1])
        minimum = [min(p["offset"][axis] for p in parts)-extent for axis, extent in enumerate([math.sqrt(3)/2, 0, 1])]
        maximum = [max(p["offset"][axis] for p in parts)+extent for axis, extent in enumerate([math.sqrt(3)/2, .4, 1])]
        require(near(actor.get("body_dimensions"), [hi-lo for lo, hi in zip(minimum, maximum)]) and near(actor.get("body_rotation"), [0, 0, 0, 1]), "dynamic bounds or fixed native yaw mismatch")
        require(near(actor.get("body_center"), [f+(lo+hi)/2 for f, lo, hi in zip(actor["feet"], minimum, maximum)]), "union center mismatch")
        require(near(actor.get("idle_mouth"), [f+o+y for f, o, y in zip(actor["feet"], parts[0]["offset"], [0, .2, 0])]), "mouth is not current physical head center")
        worm = actor.get("worm") or {}
        require(type(worm.get("head_index")) is int and worm["head_index"] == 0 and worm.get("phase") in ("Travel", "Emerging", "Exposed", "Diving") and number(worm.get("head_clearance")) and type(worm.get("exposed")) is bool, "invalid public physical phase snapshot")
    if worms:
        rendered = state.get("worm_render_parts", [])
        keys = [(p.get("actor"), p.get("index")) for p in rendered]
        require(state.get("worm_render_prisms") == len(expected_parts) and len(keys) == len(set(keys)) and set(keys) == set(expected_parts), "missing, duplicate or stale rendered segments")
        for part, key in zip(rendered, keys):
            translation, scale = expected_parts[key]
            require(near(part.get("translation"), translation) and near(part.get("scale"), scale), "rendered segment does not match current authoritative offset")
    boulders = [shot for shot in state.get("projectiles", []) if shot.get("appearance") == "Boulder"]
    for shot in boulders:
        require(shot.get("source_ability") == "WormBoulder" and number(shot.get("collision_radius")) and 0 < shot["collision_radius"] <= .5, "invalid frozen Boulder payload")
        require(all(vector(shot.get(key)) for key in ("position", "previous_position", "velocity")) and number(shot.get("age")) and shot["age"] >= 0, "invalid frozen Boulder pose")
    if worms or boulders:
        require(state.get("boulder_render_count") == len(boulders), "missing Boulder render meshes")
    if requested and phase in ("encounter-first", "encounter-third"):
        require(any(a.get("id") in state.get("visible_subjects", []) and a["worm"]["exposed"] for a in worms), "player composition has no exposed visible head")
    if not phase.startswith("encounter-worm-"):
        return
    reached = state.get("phase_reached_frame")
    require(type(reached) is int and state.get("frame", 0) >= reached+4, "phase lacks four frozen render frames")
    alive = [a for a in worms if a.get("hp", 0) > 0]
    if phase == "encounter-worm-boulder":
        require(any(shot.get("owner") == actor.get("id") and shot["age"] > 0 and any(abs(p-c) > dimension/2+shot["collision_radius"] for p, c, dimension in zip(shot["position"], actor["body_center"], actor["body_dimensions"])) for actor in worms for shot in boulders), "no real released Boulder fully outside its owner bounds")
    elif phase == "encounter-worm-buried":
        require(any(a["worm"]["phase"] == "Travel" and not a["worm"]["exposed"] and isinstance(a.get("head_earth"), dict) and type(a["head_earth"].get("substance")) is int and a["head_earth"]["substance"] > 0 and isinstance(a["head_earth"].get("position"), dict) for a in alive), "no naturally buried travelling head")
    elif phase == "encounter-worm-emerging":
        require(any(a["worm"]["phase"] == "Emerging" and .1 <= a["worm"]["head_clearance"] < .35 and a["body_hex_prisms"][0]["offset"][1]-min(p["offset"][1] for p in a["body_hex_prisms"]) > .5 for a in alive), "no natural tapered emergence")
    elif phase == "encounter-worm-windup":
        require(any(a["worm"]["exposed"] and (a.get("attack") or {}).get("kind") == "WormBoulder" and (a.get("attack") or {}).get("phase") == "Windup" and .3 <= (a.get("attack") or {}).get("progress", -1) <= .7 for a in alive), "no exposed natural Boulder windup")
    elif phase == "encounter-worm-body":
        require(alive, "no living segmented subject")
    elif phase in ("encounter-worm-converted-earth", "encounter-worm-reset"):
        evidence = state.get("worm_capture") or {}
        conversion = evidence.get("conversion") or {}
        changed = conversion.get("changed", [])
        current = evidence.get("current_cells", [])
        require(conversion.get("actor") in {a.get("id") for a in worms} and 0 < len(changed) <= 64 and len(current) == len(changed) and evidence.get("accepted_outcomes", 0) > 0, "missing bounded acknowledged conversion")
        for key in ("generation", "sequence", "frame", "tick", "revision"):
            require(type(conversion.get(key)) is int and conversion[key] >= 0, "invalid conversion provenance")
        require(type(evidence.get("dirt")) is int and evidence["dirt"] > 0, "invalid published dirt material")
        positions = [json.dumps(c.get("position"), sort_keys=True) for c in changed]
        require(len(positions) == len(set(positions)), "duplicate converted cells")
        reset = phase.endswith("reset")
        for before, now in zip(changed, current):
            position = before.get("position") or {}
            coord = position.get("coord") or {}
            require(set(position) == {"coord", "level"} and set(coord) == {"q", "r"} and all(type(coord.get(axis)) is int for axis in ("q", "r")) and type(position.get("level")) is int, "invalid converted voxel identity")
            require(type(before.get("before")) is int and before["before"] > 0, "invalid original material")
            require(before.get("position") == now.get("position") and vector(before.get("center")), "conversion/current cell correspondence mismatch")
            for key in ("health_before", "health_after"):
                health = before.get(key)
                require(isinstance(health, list) and len(health) == 2 and all(type(h) is int for h in health) and 0 < health[0] <= health[1] <= 255, "invalid acknowledged material health")
            require(before.get("before") != evidence.get("dirt"), "receipt names an unchanged dirt cell as conversion")
            expected_material = before.get("before") if reset else evidence.get("dirt")
            expected_health = None if reset or before["health_after"][0] == before["health_after"][1] else before["health_after"]
            require(now.get("material") == expected_material and now.get("published_health") == expected_health, "published material/health disagrees with conversion or reset")
        if reset:
            key = evidence.get("reset_key") or {}
            require(key.get("key") == "R" and type(key.get("frame")) is int and key["frame"] >= conversion["frame"]+4 and key["frame"] <= reached, "missing ordinary post-conversion R-key reset")
            require(evidence.get("current_generation") == conversion["generation"]+1 and evidence.get("current_revision") != conversion["revision"] and evidence.get("restored_revision") == evidence.get("current_revision") and state.get("started") is False and state.get("paused") is True, "reset did not restore a new ready round")
        else:
            require(evidence.get("current_generation") == conversion["generation"] and evidence.get("current_revision", -1) >= conversion["revision"], "conversion belongs to another generation/publication")
            surface = evidence.get("exposed_surface") or {}
            require(surface.get("position") in [c.get("position") for c in changed] and vector(surface.get("top_center")) and vector(surface.get("camera")), "missing correlated exposed converted surface")
            require(surface.get("revision") == evidence.get("current_revision") and surface.get("frame") == reached and surface["frame"] >= conversion["frame"], "stale exposed-surface composition")
            require(surface["camera"][1] > surface["top_center"][1] and abs(surface["camera"][0]-surface["top_center"][0]) < .001 and abs(surface["camera"][2]-surface["top_center"][2]) < .001, "conversion camera is not over the checked exposed top")
    else:
        raise RuntimeError("Unknown Worm phase view: " + phase)


def validate_wisp_performance_state(state: dict, view: str) -> dict | None:
    """Admit a sustained autonomous load and summarize real timings, never FPS/GPU."""
    if view != "observer-wisp-stress":
        return None

    def fail(message: str) -> None:
        raise RuntimeError(f"Wisp performance: {message}")

    def finite(value: object) -> bool:
        return type(value) in (int, float) and math.isfinite(value) and value >= 0

    def distribution(values: list[float]) -> dict:
        ordered = sorted(values)
        if not ordered:
            return {"samples": 0}
        def percentile(fraction: float) -> float:
            return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]
        return {"samples": len(ordered), "mean_ms": sum(ordered) / len(ordered),
                "p50_ms": percentile(.5), "p95_ms": percentile(.95),
                "p99_ms": percentile(.99), "max_ms": ordered[-1]}

    fixture = state.get("wisp_stress")
    if not isinstance(fixture, dict) or fixture.get("fixture") != "synthetic-wisp-hp-1000":
        fail("missing explicit synthetic fixture label")
    if fixture.get("loaded_from_config") is not True:
        fail("authored tuning must load and validate before applying synthetic HP")
    nominal = fixture.get("nominal_hp")
    if not finite(nominal) or not 0 < nominal <= 1000 or fixture.get("applied_hp") != 1000:
        fail("missing loaded nominal HP or validated synthetic HP 1000")
    if fixture.get("actor_hp_mutation") is not False or fixture.get("injected_terrain_impacts") is not False:
        fail("this fixture must use pre-admission tuning and ordinary attacks")
    if fixture.get("warmup_ticks") != 120 or fixture.get("requested_ticks") != 1440:
        fail("requires exactly 1440 ticks with the first 120 excluded")
    expected_setup = {"control": "Spectator", "player_recipe": None, "seed": 1, "tick_limit": 1440,
                      "rosters": [{"team": team, "parties": [["Wisp"] * 12]} for team in (1, 2)]}
    if state.get("battle_setup") != expected_setup or state.get("selection", {}).get("map") not in {"Duel", "Fort"}:
        fail("accepted setup must be the fixed seed-1 Fort/Duel 12-vs-12 roster")
    summary = state.get("battle_summary") or {}
    teams = summary.get("teams", [])
    if summary.get("seed") != 1 or summary.get("ticks") != 1440 or summary.get("result") != "Timeout" or state.get("tick") != 1440:
        fail("sustained workload did not reach its actual 1440-tick timeout")
    if len(teams) != 2 or {team.get("team") for team in teams} != {1, 2}:
        fail("accepted summary must contain both teams")
    actors = state.get("actors", [])
    if len(actors) != 24 or {a.get("id") for a in actors} != set(range(24)) or state.get("human_actor_id") is not None:
        fail("all 24 actual actors, including actor 0, are required")
    for team in teams:
        members = [a for a in actors if a.get("team") == team["team"]]
        if team.get("initial") != 12 or team.get("living") != 12 or team.get("max_hp") != 12000 or len(members) != 12:
            fail("admitted team HP/counts disagree with 12 actors at HP 1000")
        if any(a.get("species") != "Wisp" or a.get("max_hp") != 1000 or not finite(a.get("hp")) or not 0 < a["hp"] <= 1000 or a.get("flying") is not True for a in members):
            fail("every actor must remain a living flying Wisp with admitted HP 1000")
        if {a.get("flight_layer") for a in members} != {0, 1}:
            fail("each team must retain both published flight layers")
        if not finite(team.get("hp")) or abs(team["hp"] - sum(a["hp"] for a in members)) > .1:
            fail("summary remaining HP disagrees with its actual actors")
    rows = fixture.get("rows", [])
    if not isinstance(rows, list) or len(rows) != 1440 or [r.get("tick") for r in rows] != list(range(1, 1441)):
        fail("requires contiguous, unrepeated simulation rows 1 through 1440")
    for row in rows:
        if type(row.get("frame")) is not int or row["frame"] <= 0 or not finite(row.get("cpu_ms")):
            fail("invalid containing frame or measured tick CPU interval")
        if any(type(row.get(key)) is not bool for key in ("terrain_publication", "damage_outcome")) or type(row.get("destroyed_voxels")) is not int or row["destroyed_voxels"] < 0:
            fail("missing actual publication/damage/destruction measurements")
    if any(a["frame"] > b["frame"] for a, b in zip(rows, rows[1:])):
        fail("simulation rows have out-of-order app frames")
    measured = rows[120:]
    for row in measured:
        load = row.get("load", {})
        if any(load.get(key) != value for key, value in (("living_wisps", 24), ("flying_wisps", 24), ("active_parties", 2), ("unassigned_layers", 0))):
            fail("measured tick lost part of its 24-Wisp all-active flight load")
        layers = load.get("team_layers")
        if not isinstance(layers, list) or len(layers) != 2 or any(not isinstance(team, list) or len(team) != 2 or any(type(n) is not int or n <= 0 for n in team) or sum(team) != 12 for team in layers):
            fail("measured tick lacks both occupied flight layers on each team")
        if any(type(load.get(key)) is not int or load[key] < 0 for key in ("projectiles", "windups")):
            fail("missing projectile/windup load counters")
    if not any(row["load"]["projectiles"] for row in measured) or not any(row["load"]["windups"] for row in measured):
        fail("ordinary autonomous attacks did not exercise the measured workload")
    wall = state.get("app_frame_wall_intervals_ms", [])
    if not isinstance(wall, list) or not wall or not all(finite(value) for value in wall):
        fail("missing real Instant app-frame intervals")
    warmup_frames = {row["frame"] for row in rows[:120]}
    frames = sorted({row["frame"] for row in measured} - warmup_frames)
    if not frames or max(frames) > len(wall):
        fail("missing the interval containing a measured app frame")
    intervals = [wall[frame - 1] for frame in frames]
    ticks = [row["cpu_ms"] for row in measured]
    publication = [row["cpu_ms"] for row in measured if row["terrain_publication"]]
    destruction = [row["cpu_ms"] for row in measured if row["destroyed_voxels"]]
    flush = fixture.get("terminal_publication")
    if not isinstance(flush, dict) or flush.get("tick") != 1440 or not finite(flush.get("cpu_ms")) or type(flush.get("terrain_publication")) is not bool:
        fail("missing separately measured terminal publication without a living tick")
    return {"fixture": fixture["fixture"], "nominal_hp": nominal, "applied_hp": 1000,
            "warmup_ticks_excluded": 120, "measured_ticks": len(measured),
            "tick_cpu": distribution(ticks), "tick_over_8_333_ms": sum(t > 1000 / 120 for t in ticks),
            "app_frame_wall_interval": distribution(intervals), "app_intervals_over_16_667_ms": sum(t > 1000 / 60 for t in intervals),
            "mixed_warmup_frames_excluded": len({r["frame"] for r in measured} & warmup_frames),
            "publication_ticks": len(publication), "publication_tick_cpu": distribution(publication),
            "damage_outcome_ticks": sum(r["damage_outcome"] for r in measured),
            "destroyed_voxels": sum(r["destroyed_voxels"] for r in measured),
            "destruction_tick_cpu": distribution(destruction), "terminal_publication": flush,
            "peak_projectiles": max(r["load"]["projectiles"] for r in measured),
            "peak_windups": max(r["load"]["windups"] for r in measured),
            "boundary": "Real CPU and Instant app-Update start-to-start intervals; no GPU/vsync/FPS, ordinary balance, movement or static approval claim. Zero publications do not exercise terrain destruction; use the separate all-ten Seven Regions/destruction workloads."}


def validate_encounter_stress_home_leashes(rows):
    """Every synthetic tick must disclose the admission-time leash override."""
    if not isinstance(rows, list) or not rows or any(
        not isinstance(row, dict)
        or not isinstance(row.get("stimulus"), dict)
        or row["stimulus"].get("home_leashes") != [150.0, 150.0, 150.0]
        for row in rows
    ):
        raise RuntimeError("Synthetic encounter stress requires explicit ground/Shadow/Dragon home leashes of 150 units on every tick.")


def validate_terminal_menu_state(state: dict, view: str) -> None:
    """Explicit knockout fixtures prove result-menu presentation, not combat outcomes."""
    expected = {"terminal-win": "win", "terminal-defeat": "defeat"}.get(view)
    if expected is None:
        return
    if (state.get("started") is not True or state.get("paused") is not True
            or state.get("terminal_menu_outcome") != expected
            or state.get("terminal_menu_fixture") != "synthetic-knockout-for-menu-presentation"):
        raise RuntimeError(f"{view} lacks its completed synthetic knockout and automatic paused menu.")
    reached = state.get("phase_reached_frame")
    if type(reached) is not int or state.get("frame", 0) < reached + 4:
        raise RuntimeError(f"{view} lacks four rendered frames after the actual terminal transition.")


def native_receipt_info(png: Path, view: str, pixels: list[int]) -> dict:
    path = png.with_suffix(".json")
    data = path.read_bytes()
    try:
        state = json.loads(data)
    except (ValueError, UnicodeDecodeError) as error:
        raise RuntimeError(f"Invalid native state receipt: {path}") from error
    if not isinstance(state, dict) or state.get("view") != view:
        raise RuntimeError(f"Native state receipt does not identify view {view}: {path}")
    if [state.get("width"), state.get("height")] != pixels or pixels != CANVAS:
        raise RuntimeError(f"Native receipt/PNG dimensions disagree with the {CANVAS} capture canvas.")
    if view.startswith("observer-") or state.get("battle_setup", {}).get("control") == "Spectator":
        setup, summary = state.get("battle_setup", {}), state.get("battle_summary")
        if setup.get("control") != "Spectator" or state.get("human_actor_id") is not None or not isinstance(summary, dict):
            raise RuntimeError(f"{view} lacks an accepted observer battle with no human actor.")
        if len(summary.get("teams", [])) != 2 or any(actor.get("species") == "Human" for actor in state.get("actors", [])):
            raise RuntimeError(f"{view} did not initialize exactly two monster teams.")
        if any(sample.get("pressed") or sample.get("held") or sample.get("released") or sample.get("jump") or sample.get("run") or any(sample.get("movement", [])) for sample in state.get("capture_inputs", [])):
            raise RuntimeError(f"{view} leaked player input into the observer battle.")
        if view == "observer-result" and summary.get("result") is None:
            raise RuntimeError("Observer result capture has no actual terminal result.")
        if view == "observer-free" and state.get("observer_camera", {}).get("mode") != "Free":
            raise RuntimeError("Observer free-camera capture did not enter Free mode.")
        if view.startswith("observer-") and view not in {"observer-start", "observer-paused"}:
            reached = state.get("composition_reached_frame") if view.startswith("observer-close") else state.get("phase_reached_frame")
            if not isinstance(reached, int) or state.get("frame", 0) < reached + 4:
                raise RuntimeError(f"{view} lacks its requested simulation boundary and four rendered frames.")
        if view.startswith("observer-close"):
            subjects = set(state.get("visible_subjects", []))
            teams = {actor.get("team") for actor in state.get("actors", []) if actor.get("id") in subjects and actor.get("hp", 0) > 0}
            if teams != {team.get("team") for team in summary.get("teams", [])}:
                raise RuntimeError("Close observer capture must admit a useful visible subject from each living team.")
            samples = state.get("observer_camera_inputs", [])
            if not any(sample.get("wheel", 0) > 0 for sample in samples) or (view.endswith("-rear") and not any(abs(sample.get("look", [0])[0]) > 0 for sample in samples)):
                raise RuntimeError("Close observer capture lacks its ordinary wheel/azimuth input history.")
        if view == "observer-performance" and summary.get("ticks", 0) < 3600 and summary.get("result") is None:
            raise RuntimeError("Observer performance must reach 3600 ticks or an actual terminal result.")
    if view in BOT_VIEWS:
        actors = state.get("round_summary", {}).get("actors", [])
        if len(actors) != 2 or not any(actors[1].get("casts", [])):
            raise RuntimeError(f"{view} did not exercise actual bot releases.")
        if not state.get("bot_debug", {}).get("decisions") or not state.get("terrain_outcomes"):
            raise RuntimeError(f"{view} did not exercise bot decisions and terrain damage publication.")
    if view in CHARGE_VIEWS and ("charge-" in view or "armed" in view or "partial-preview" in view):
        human = next((actor for actor in state.get("actors", []) if actor.get("id") == 0), {})
        charge = human.get("charge")
        expected_spell = "Shield" if view.startswith("shield") else "Fireball" if view.startswith("fireball") else "AreaBlast"
        if not isinstance(charge, dict) or charge.get("spell") != expected_spell:
            raise RuntimeError(f"{view} did not retain the expected authoritative {expected_spell} charge.")
        progress = charge.get("progress")
        if not isinstance(progress, (int, float)) or not 0 <= progress <= 1:
            raise RuntimeError(f"{view} has no valid charge progress in its native receipt.")
        if "charge-partial" in view and not 0.35 <= progress <= 0.65:
            raise RuntimeError(f"{view} missed the partial-charge capture interval: {progress}.")
        if ("charge-full" in view or "partial-preview" in view) and progress < 0.99:
            raise RuntimeError(f"{view} did not reach full charge: {progress}.")
        samples = state.get("capture_inputs", [])
        if not any(sample.get("pressed") for sample in samples) or any(sample.get("released") for sample in samples):
            raise RuntimeError(f"{view} must retain a recorded press/hold sequence without a release.")
        if "partial-preview" in view:
            fixture = state.get("fixture_voxels", [])
            preview = state.get("preview", {})
            if len(fixture) != 2 or not preview.get("valid") or not preview.get("wall_voxels"):
                raise RuntimeError(f"{view} is missing its world-published partial shield fixture.")
            if any(voxel in preview["wall_voxels"] for voxel in fixture):
                raise RuntimeError(f"{view} preview failed to omit its occupied fixture slots.")
    if state.get("selection", {}).get("map") == "Fort" and (view in {"encounter-first", "encounter-third"} or view.startswith("encounter-body")):
        reached = state.get("composition_reached_frame")
        if not state.get("approach_complete") or not state.get("visible_subjects") or not isinstance(reached, int) or state.get("frame", 0) < reached + 4:
            raise RuntimeError(f"{view} requires a completed Fort approach, visible subject, and four stable composition frames.")
    phase_views = {"encounter-windup", "encounter-breath", "encounter-swipe", "encounter-barrier", "encounter-aura", "encounter-fireball"}
    phase_view = view.removesuffix("-rear")
    if phase_view in phase_views:
        reached = state.get("phase_reached_frame")
        if not isinstance(reached, int) or state.get("frame", 0) < reached + 4:
            raise RuntimeError(f"{view} lacks a frozen authoritative phase and four render frames.")
        actors = state.get("actors", [])
        attacks = [actor.get("attack") for actor in actors if actor.get("id") != 0 and actor.get("attack")]
        valid = {
            "encounter-barrier": bool(state.get("barriers")),
            "encounter-aura": bool(state.get("auras")),
            "encounter-breath": any(a.get("kind") == "FireCone" and a.get("phase") == "Active" for a in attacks),
            "encounter-swipe": any(a.get("kind") == "Swipe" and a.get("phase") == "Windup" for a in attacks),
            "encounter-windup": any(a.get("phase") == "Windup" for a in attacks),
            "encounter-fireball": any(actor.get("id") != 0 and (actor.get("charge") or {}).get("spell") == "Fireball" for actor in actors),
        }[phase_view]
        if not valid:
            raise RuntimeError(f"{view} native state does not contain its requested ability phase.")
    if view == "encounter-stress":
        rows = state.get("stress_ticks", [])
        if not state.get("synthetic_fixture") or len(rows) != 3600:
            raise RuntimeError("Synthetic performance receipt requires its fixture label and 3600 ticks.")
        seven = state.get("selection", {}).get("map") == "Seven Regions"
        party_count = 3 if seven else 1
        enemy_count = 17 if seven else {"Dragon": 1, "Goblins": 10, "Shaman party": 6, "Shadow": 1}.get(state.get("selection", {}).get("encounter"), 0)
        validate_encounter_stress_home_leashes(rows)
        measured = rows[120:]
        all_active = [row for row in measured if row.get("active_parties") == party_count and row.get("living_enemies") == enemy_count]
        if len(all_active) < 2400:
            raise RuntimeError(f"Synthetic capture has only {len(all_active)} sustained all-enemy-active ticks; need 2400.")
        if not any(row.get("terrain_publication") for row in measured) or not any(row.get("damage_outcome") for row in measured) or not sum(row.get("destroyed_voxels", 0) for row in measured):
            raise RuntimeError("Synthetic capture did not retain measured terrain damage and destruction publication pressure.")
        wall = state.get("app_frame_wall_intervals_ms", [])
        if not wall or not all(isinstance(value, (int, float)) and 0 <= value < float("inf") for value in wall):
            raise RuntimeError("Synthetic capture lacks valid real app-frame wall intervals.")
    validate_terminal_menu_state(state, view)
    validate_golem_state(state, view)
    validate_wisp_state(state, view)
    validate_worm_state(state, view)
    wisp_performance = validate_wisp_performance_state(state, view)
    ready_frame = state.get("render_ready_frame")
    if not isinstance(ready_frame, int) or state.get("frame", 0) < ready_frame + 4:
        raise RuntimeError(f"{view} did not wait four frames after render assets became ready.")
    if state.get("liquid_phase_seconds") != 0.0:
        raise RuntimeError(f"{view} did not capture liquid presentation frozen at phase zero.")
    if state.get("static_spans", 0) and not state.get("authored_objects", 0):
        raise RuntimeError(f"{view} has published static props without authored object instances.")
    if state.get("authored_objects", 0) and not state.get("object_render_chunks", 0):
        raise RuntimeError(f"{view} contains authored instances without render chunks.")
    return {"file": path.name, "sha256": digest(data), "bytes": len(data),
            "frame": state.get("frame"), "tick": state.get("tick"),
            **({"wisp_performance": wisp_performance} if wisp_performance is not None else {})}


def capture(args: argparse.Namespace) -> int:
    views = BOT_VIEWS if args.bot_review else CHARGE_VIEWS if args.charge_review else MENU_VIEWS if args.menu_review else VIEWS
    matrix = "arena-bot-v1" if args.bot_review else "arena-charge-v1" if args.charge_review else "arena-menu-v3-terminal" if args.menu_review else MATRIX
    observer_matrix = args.spectator_review or args.spectator_performance
    if (args.golem_review or args.wisp_review or args.wisp_performance or args.worm_review) and any(value is not None and value is not False for value in (args.map, args.encounter, args.spectator, args.team_a, args.team_b, args.seed, args.tick_limit)):
        raise RuntimeError("The creature matrix defines its player and observer recipes; use --view to select entries.")
    battle_environment(args, args.map or "fort", matrix=observer_matrix)
    if observer_matrix and (args.map or args.encounter):
        raise RuntimeError("Spectator matrices define maps and encounters; use --view to select entries.")
    if (args.encounter_review or args.performance_review) and (args.spectator or args.team_a or args.team_b or args.seed is not None or args.tick_limit is not None):
        raise RuntimeError("Player encounter matrices do not accept observer options.")
    if (args.encounter_review or args.performance_review) and (args.map or args.encounter):
        raise RuntimeError("Encounter/performance matrices define each recipe; use --view to select entries.")
    entries = list(PERFORMANCE_VIEWS) if args.performance_review else list(ENCOUNTER_VIEWS) if args.encounter_review else [(view, view, args.map or "duel", args.encounter or "shadow", None) for view in views]
    if observer_matrix or args.spectator:
        if args.bot_review or args.charge_review or args.menu_review:
            raise RuntimeError("Use --spectator-review for the observer menu/camera matrix.")
        entries = list(OBSERVER_PERFORMANCE_VIEWS if args.spectator_performance else OBSERVER_VIEWS)
        if args.spectator and not observer_matrix and args.map:
            entries = [entry for entry in entries if entry[2] == args.map]
        matrix = "arena-spectator-performance-v1" if args.spectator_performance else "arena-spectator-v2-close"
    if args.performance_review:
        matrix = "arena-performance-v2-synthetic-extended-leashes"
    elif args.encounter_review:
        matrix = "arena-encounters-v2-multi-angle"
    if args.golem_review:
        entries = list(GOLEM_VIEWS)
        matrix = "arena-golem-v3-pressure-phases"
    if args.wisp_review:
        entries = list(WISP_VIEWS)
        matrix = "arena-wisp-v1-natural-phases"
    if args.worm_review:
        entries = list(WORM_VIEWS)
        matrix = "arena-worm-v4-clear-earth"
    if args.wisp_performance:
        entries = list(WISP_PERFORMANCE_VIEWS)
        matrix = "arena-wisp-performance-v1-synthetic"
    if args.view:
        requested = set(args.view)
        unknown = requested - {entry[0] for entry in entries}
        if unknown:
            raise RuntimeError(f"Unknown views for this matrix: {sorted(unknown)}")
        entries = [entry for entry in entries if entry[0] in requested]
        matrix += "-focused"
    output = args.output
    if not output.is_absolute():
        raise RuntimeError("--output must be an absolute path to a new directory.")
    if output.exists() or output.is_symlink():
        raise RuntimeError(f"Refusing to reuse an existing capture directory: {output}")
    output = output.resolve()
    if output.is_relative_to(ROOT):
        ignored = subprocess.run(
            ("git", "check-ignore", "--quiet", str(output)), cwd=ROOT, check=False,
        ).returncode == 0
        if not ignored:
            raise RuntimeError("Capture output inside the checkout must be Git-ignored (use .context/).")
    env, removed = environment(args.target_dir)
    initial, staged, unstaged = source_state()
    output.mkdir(parents=True, exist_ok=False)
    # The state-derived child is the actual pack; the requested directory is never reused.
    state_id = initial["head"]
    if initial["dirty"]:
        state_id += "-dirty-" + initial["state_sha256"][:12]
    pack = output / f"{state_id}-{matrix}"
    pack.mkdir()
    (pack / "staged.patch").write_bytes(staged)
    (pack / "unstaged.patch").write_bytes(unstaged)
    write_json(pack / "source-state.json", initial)
    receipt = {
        "schema_version": 1, "matrix": matrix, "pack": str(pack), "repository": str(ROOT),
        "started_at": utc_now(), "source": initial,
        "source_label": "UNAPPROVABLE-DIRTY" if initial["dirty"] else "COMMITTED-CANDIDATE",
        "scenario": "Spell Combat Arena / explicit deterministic recipes",
        "terrain_seed_note": "Each frame records its accepted recipe and fixed seed.",
        "scenario_correction": "Worm buried view requires actual Travel plus published head-center earth. Conversion view uses ordinary Duel Worm/Goblins and waits for a correlated exposed dirt top after actor body and surface decoration clear it; Fort retains the ordinary reset comparison. Windup uses the exposed physical head warning material." if args.worm_review else "Duel observer Golem vs Dragon: native 3710941 paired corpus exercised GolemLaser in 16/16 Dragon rows and 0/16 Shadow rows. Ordinary rosters/seed 1; no injected state or weakened phase guards." if args.golem_review else None,
        "capture_method": "windowless Bevy arena image-target hook",
        "terminal_menu_note": "terminal-win/terminal-defeat explicitly set fixture HP to zero; normal authority computes the result and opens the menu. These rows establish presentation only, not naturally won/lost combat." if any(row[1].startswith("terminal-") for row in entries) else None,
        "logical_canvas": CANVAS, "device_scale": 1.0,
        "changed_surfaces": ["dynamic head-first native Worm segments", "opaque-earth occlusion", "Boulder windup and frozen projectile", "seven-button Fort menu", "acknowledged dirt conversion and key reset"] if args.worm_review else ["24 autonomous Wisps", "both flight layers", "native app-frame and tick load"] if args.wisp_performance else ["one-prism Wisp", "glow and dim-light comparisons", "frozen Ember appearance", "six-button Fort menu", "observer swarm labels"] if args.wisp_review else ["seven-prism stone body", "independent face", "charge/tracking/beam", "spherical slam warning", "frontal Stone Swipe", "Fort party selector", "observer Golem roster"] if args.golem_review else ["observer mode and rosters", "orbit/free camera", "team body colors", "observer HUD", "terminal results"] if (observer_matrix or args.spectator) else ["map selectors", "authored map terrain and objects", "creature models", "windups", "breath", "barrier", "aura", "party count"] if args.encounter_review else ["charge bar", "release guidance", "partial shield footprint", "ready screen", "paused menu", "actor cameras"] if args.charge_review else ["ready screen", "paused menu", "synthetic win/defeat result menus", "HUD key guidance"] if args.menu_review else ["terrain", "actor cameras", "cover", "spell effects", "HUD", "tuning", "ready screen"],
        "expected_views": [entry[0] for entry in entries], "mechanical_status": "INCOMPLETE",
        "static_review": "NOT_AN_APPROVAL_PACK" if (args.performance_review or args.spectator_performance or args.wisp_performance) else "UNREVIEWED", "human_motion": "NOT_MEASURED_SYNTHETIC" if (args.performance_review or args.wisp_performance) else "OBSERVER-CAMERA-MOTION-PENDING" if (observer_matrix or args.spectator) else "HUMAN-MOTION-PENDING",
        "performance_fixture": "Synthetic validated Wisp HP 1000 before admission, 12 vs 12 for 1440 ticks; authored nominal HP retained per native receipt. No actor HP mutation or injected impacts. Actual zero terrain publications are valid; separate Seven Regions/destruction fixtures cover that workload." if args.wisp_performance else "Synthetic extra-HP party visits with validated 150-unit ground/Shadow/Dragon home leashes before admission. Authored search durations, sight, activation, movement and attacks; no ordinary movement or human balance evidence." if args.performance_review else "Ordinary seeded autonomous battle; real app-frame wall intervals, no GPU or vsync measurement." if args.spectator_performance else None,
        "human_route": "Choose both teams and map; start, pan/orbit/zoom, switch free camera, move near walls, pause/focus/resume, observe actual result, reset and switch back to Play. Camera controls never command a creature." if (observer_matrix or args.spectator) else "Select and restart every map and Fort encounter, traverse the three dry Seven Regions approaches, observe windups/breath/barrier/aura and party completion. Move, jump, sprint, look near walls, toggle camera; tap, partially charge and fully charge Shield/Fireball, release Area Blast, cancel holds with pause/focus/spell changes, and reset.",
        "gameplay_evidence": "Not established by captures; use typed tests and simulation receipts.",
        "inherited_capability_names_removed": removed,
        "environment": {key: env[key] for key in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS")},
        "frames": [],
    }
    write_json(pack / "receipt.json", receipt)
    print(f"Windowless capture pack: {pack}", flush=True)
    try:
        seen = {}
        for name, view, arena_map, encounter, focus in entries:
            if source_state()[0] != initial:
                raise RuntimeError("Source changed during capture; this pack is stale.")
            png = pack / f"{name}.png"
            frame_env = dict(env, HEX_ARENA_CAPTURE=str(png), HEX_ARENA_VIEW=view,
                             HEX_ARENA_MAP=arena_map, HEX_ARENA_ENCOUNTER=encounter)
            row_args = args
            if args.golem_review and arena_map == "duel":
                row_args = argparse.Namespace(**vars(args))
                row_args.spectator = True
                row_args.team_a, row_args.team_b = GOLEM_OBSERVER_PRESETS
            if args.wisp_review and name in WISP_OBSERVER_RECIPES:
                row_args = argparse.Namespace(**vars(args))
                row_args.spectator = True
                row_args.team_a, row_args.team_b = WISP_OBSERVER_RECIPES[name]
            if args.worm_review and arena_map == "duel":
                row_args = argparse.Namespace(**vars(args))
                row_args.spectator = True
                row_args.team_a, row_args.team_b = WORM_OBSERVER_PRESETS
            if args.wisp_performance:
                row_args = argparse.Namespace(**vars(args))
                row_args.spectator = True
                row_args.team_a = row_args.team_b = "wisps-12"
                row_args.seed, row_args.tick_limit = 1, 1440
            frame_env.update(battle_environment(row_args, arena_map, matrix=observer_matrix, result=view == "observer-result"))
            if focus:
                frame_env["HEX_ARENA_FOCUS"] = focus
            frame = {
                "name": name, "view": view, "map": arena_map, "encounter": encounter, "terrain_seed": SEEDS[arena_map], "focus_anchor": focus, "started_at": utc_now(), "static_review": "NOT_AN_APPROVAL_PACK" if (args.performance_review or args.spectator_performance or args.wisp_performance) else "UNREVIEWED",
                "command": ["cargo", *CARGO_ARGS], "cwd": str(ROOT),
                "capabilities": {key: value for key, value in frame_env.items() if key.startswith("HEX_ARENA_")},
                "log": f"{name}.log", "mechanical_status": "INCOMPLETE",
            }
            receipt["frames"].append(frame)
            write_json(pack / "receipt.json", receipt)
            print(f"Capturing {name}…", flush=True)
            frame["exit_code"] = run_cargo(frame_env, pack / frame["log"], args.timeout)
            if frame["exit_code"]:
                raise RuntimeError(f"{view} exited with code {frame['exit_code']}; inspect {frame['log']}.")
            frame.update(png_info(png))
            frame["native_state_receipt"] = native_receipt_info(png, view, frame["physical_pixels"])
            native_state = json.loads(png.with_suffix(".json").read_text())
            validate_capture_setup(native_state, arena_map, encounter, frame_env)
            frame.update(logical_canvas=CANVAS, device_scale=1.0)
            if frame["sha256"] in seen:
                raise RuntimeError(f"Unexpected duplicate frames: {view} and {seen[frame['sha256']]}.")
            seen[frame["sha256"]] = name
            frame.update(mechanical_status="CAPTURED", finished_at=utc_now())
            write_json(pack / "receipt.json", receipt)
        final = source_state()[0]
        receipt["source_after"] = final
        if final != initial:
            raise RuntimeError("Source changed during capture; this pack is stale.")
        receipt["mechanical_status"] = "COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        receipt["mechanical_status"] = "BLOCKED"
        receipt["error"] = str(error) or "Interrupted"
        raise
    finally:
        receipt["finished_at"] = utc_now()
        write_json(pack / "receipt.json", receipt)
        for frame in receipt["frames"]:
            log = pack / frame["log"]
            if log.is_file():
                frame["log_sha256"] = digest(log.read_bytes())
            write_json(pack / f"{frame['name']}.receipt.json", frame)
        write_json(pack / "receipt.json", receipt)
    print("Synthetic performance capture complete. No movement, human balance, GPU, or vsync claim." if (args.performance_review or args.wisp_performance) else "Capture matrix complete. Static review: UNREVIEWED. Native motion: HUMAN-MOTION-PENDING.")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    launch = commands.add_parser("launch", help="Explicitly open the native playable arena through Cargo.")
    captures = commands.add_parser("capture", help="Capture all 23 views without a native window.")
    for command in (launch, captures):
        command.add_argument("--map", choices=MAPS, help="Map recipe (launch: fort; legacy capture: duel).")
        command.add_argument("--encounter", choices=ENCOUNTERS, help="Duel/Fort enemy party (launch: Shadow on Duel, Dragon on Fort; capture: Shadow).")
        command.add_argument("--spectator", action="store_true", help="Observe autonomous monster teams; Fort or Duel only.")
        command.add_argument("--team-a", choices=ENCOUNTERS, help="Cyan roster preset (spectator default: goblins).")
        command.add_argument("--team-b", choices=ENCOUNTERS, help="Amber roster preset (spectator default: shaman-party).")
        command.add_argument("--seed", type=int, help="Deterministic observer decision seed (default: 1).")
        command.add_argument("--tick-limit", type=int, help="Observer simulation limit at 120 Hz (default: 14400).")
        command.add_argument("--target-dir", type=Path, default=DEFAULT_TARGET,
                             help="Explicit shared Cargo target directory (absolute path).")
    captures.add_argument("--output", type=Path, required=True,
                          help="New absolute parent directory; receives a state-named capture pack.")
    captures.add_argument("--timeout", type=float, default=300,
                          help="Maximum seconds per capture, including any Cargo work (default: 300).")
    captures.add_argument("--view", action="append", help="Capture only a named matrix entry; repeat for multiple entries.")
    review = captures.add_mutually_exclusive_group()
    review.add_argument("--wisp-performance", action="store_true", help="Two separate synthetic Fort/Duel 12-vs-12 Wisp workloads: validated HP 1000, 1440 ticks, first 120 excluded; no injected impacts or actor HP mutation.")
    review.add_argument("--worm-review", action="store_true", help="Thirteen ordinary Worm menu, full Fort, body, emergence, windup, Boulder, acknowledged earth conversion and R-key reset views.")
    review.add_argument("--wisp-review", action="store_true", help="Twelve Wisp body, dim-light, windup, Ember, layered swarm and menu views from ordinary accepted recipes.")
    review.add_argument("--golem-review", action="store_true", help="Fourteen natural Fort-player and Duel Golem-vs-Dragon observer body, charge, beam, slam and Stone Swipe views; missing natural phase admission fails.")
    review.add_argument("--spectator-review", action="store_true", help="Fourteen Fort/Duel observer menu, whole-map orbit, close two-azimuth, free and terminal views.")
    review.add_argument("--spectator-performance", action="store_true", help="Fort/Duel ordinary observer frame intervals until 3600 ticks or a terminal result; no synthetic HP or movement.")
    review.add_argument("--performance-review", action="store_true", help="Capture five synthetic 3600-tick extra-HP workloads with validated 150-unit home leashes: four Fort presets and all seventeen Seven Regions enemies.")
    review.add_argument("--encounter-review", action="store_true", help="Capture 27 map, creature, attack-phase, and selector views, including opposite barrier/aura azimuths.")
    review.add_argument("--menu-review", action="store_true",
                        help="Capture eight ready/menu/HUD views, including explicitly synthetic win/defeat menu fixtures.")
    review.add_argument("--charge-review", action="store_true",
                        help="Capture 16 charge/release, clipped shield, ready/menu, and actor-camera views.")
    review.add_argument("--bot-review", action="store_true",
                        help="Run up to ten seconds of real bot combat in each camera and retain renderer/tick timings.")
    args = parser.parse_args(argv)
    try:
        if not args.target_dir.is_absolute():
            raise RuntimeError("--target-dir must be absolute.")
        if args.command == "capture":
            if not 0 < args.timeout < float("inf"):
                raise RuntimeError("--timeout must be a finite positive number.")
            return capture(args)
        battle_env = battle_environment(args, args.map or "fort")
        env, _ = environment(args.target_dir)
        env.update(battle_env)
        env.update(HEX_ARENA_MAP=args.map or "fort", HEX_ARENA_ENCOUNTER=args.encounter or ("shadow" if args.map == "duel" else "dragon"))
        print(f"Opening native Spell Combat Arena: {shlex.join(('cargo', *CARGO_ARGS))}", flush=True)
        return run_cargo(env, None, None)
    except KeyboardInterrupt:
        print("Arena helper interrupted.", file=sys.stderr)
        return 130
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Arena helper: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
