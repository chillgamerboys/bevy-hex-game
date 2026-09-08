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
DEFAULT_TARGET = Path(
    "/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/"
    "work/cargo-target-explore"
)
VIEWS = (
    "overview", "first", "third", "rear", "shield", "fireball", "blast", "tuning",
    "shield-compact", "shield-large", "fireball-compact", "fireball-large",
    "blast-compact", "blast-large",
    "shield-first", "shield-third", "fireball-first", "fireball-third",
    "blast-first", "blast-third", "shield-preview-first", "shield-preview-third", "start",
)
MATRIX = "arena-v5-release-casting"
MENU_VIEWS = ("start", "tuning", "first", "third", "overview", "rear")
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
ENCOUNTERS = ("dragon", "goblins", "shaman-party", "shadow", "golem", "goblin", "wisp", "wisps-2", "wisps-4", "wisps-8", "wisps-12")
PRESET_MEMBERS = {"shadow": ["Shadow"], "dragon": ["Dragon"], "goblins": ["Goblin"] * 5,
                  "shaman-party": ["Shaman", "Goblin", "Goblin", "Goblin"], "golem": ["Golem"],
                  "goblin": ["Goblin"], "wisp": ["Wisp"], **{f"wisps-{n}": ["Wisp"] * n for n in (2, 4, 8, 12)}}
PLAYER_OVERRIDES = {"golem": "Golem", "goblin": "Goblin", "wisp": "Wisp", "wisps-2": "Wisps2", "wisps-4": "Wisps4", "wisps-8": "Wisps8", "wisps-12": "Wisps12"}
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
    ("duel-golem-charge", "encounter-golem-charge", "duel", "shadow", None),
    ("duel-golem-locked", "encounter-golem-locked", "duel", "shadow", None),
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
OBSERVER_PERFORMANCE_VIEWS = tuple(
    (f"{arena_map}-observer-performance", "observer-performance", arena_map, "shadow", None)
    for arena_map in ("fort", "duel")
)


def battle_environment(args: argparse.Namespace, arena_map: str, *, matrix: bool = False, result: bool = False) -> dict[str, str]:
    if args.encounter in PLAYER_OVERRIDES and arena_map != "fort":
        raise RuntimeError("Creature player overrides require Fort; Duel supports their spectator rosters.")
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
    """Check the accepted setup, including the independent Fort player override."""
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
                "player_recipe": PLAYER_OVERRIDES.get(encounter) if not observing else None}
    if state.get("battle_setup") != expected:
        raise RuntimeError("Capture accepted a different control, roster, seed, tick limit or player recipe.")
    if observing:
        expected_members = Counter((roster["team"], species) for roster in rosters for party in roster["parties"] for species in party)
        actual_members = Counter((actor.get("team"), actor.get("species")) for actor in state.get("actors", []))
        if actual_members != expected_members:
            raise RuntimeError("Capture actual bodies differ from the accepted spectator teams.")
    elif encounter in PLAYER_OVERRIDES and Counter(actor.get("species") for actor in state.get("actors", [])) != Counter(["Human", *PRESET_MEMBERS[encounter]]):
        raise RuntimeError("Fort player override published different actual creature bodies.")


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
            if not isinstance(beam, dict) or not all(vector(beam.get(key)) for key in ("origin", "direction", "end")) or type(beam.get("locked")) is not bool:
                raise RuntimeError("Golem beam snapshot contains invalid vectors/lock state.")
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
            "encounter-golem-charge": laser and stage == "Windup" and 0.25 <= progress <= 0.55 and beam.get("locked") is False,
            "encounter-golem-locked": laser and stage == "Windup" and beam.get("locked") is True,
            "encounter-golem-beam": laser and stage == "Active" and beam.get("locked") is True,
            "encounter-golem-slam-windup": kind == "GolemSlam" and stage == "Windup" and progress >= 0.25,
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
        enemy_count = 10 if seven else {"Dragon": 1, "Goblins": 5, "Shaman party": 4, "Shadow": 1}.get(state.get("selection", {}).get("encounter"), 0)
        measured = rows[120:]
        all_active = [row for row in measured if row.get("active_parties") == party_count and row.get("living_enemies") == enemy_count]
        if len(all_active) < 2400:
            raise RuntimeError(f"Synthetic capture has only {len(all_active)} sustained all-enemy-active ticks; need 2400.")
        if not any(row.get("terrain_publication") for row in measured) or not any(row.get("damage_outcome") for row in measured) or not sum(row.get("destroyed_voxels", 0) for row in measured):
            raise RuntimeError("Synthetic capture did not retain measured terrain damage and destruction publication pressure.")
        wall = state.get("app_frame_wall_intervals_ms", [])
        if not wall or not all(isinstance(value, (int, float)) and 0 <= value < float("inf") for value in wall):
            raise RuntimeError("Synthetic capture lacks valid real app-frame wall intervals.")
    validate_golem_state(state, view)
    validate_wisp_state(state, view)
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
            "frame": state.get("frame"), "tick": state.get("tick")}


def capture(args: argparse.Namespace) -> int:
    views = BOT_VIEWS if args.bot_review else CHARGE_VIEWS if args.charge_review else MENU_VIEWS if args.menu_review else VIEWS
    matrix = "arena-bot-v1" if args.bot_review else "arena-charge-v1" if args.charge_review else "arena-menu-v2" if args.menu_review else MATRIX
    observer_matrix = args.spectator_review or args.spectator_performance
    if (args.golem_review or args.wisp_review) and any(value is not None and value is not False for value in (args.map, args.encounter, args.spectator, args.team_a, args.team_b, args.seed, args.tick_limit)):
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
        matrix = "arena-performance-v1-synthetic"
    elif args.encounter_review:
        matrix = "arena-encounters-v2-multi-angle"
    if args.golem_review:
        entries = list(GOLEM_VIEWS)
        matrix = "arena-golem-v2-dragon-phases"
    if args.wisp_review:
        entries = list(WISP_VIEWS)
        matrix = "arena-wisp-v1-natural-phases"
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
        "scenario_correction": "Duel observer Golem vs Dragon: native 3710941 paired corpus exercised GolemLaser in 16/16 Dragon rows and 0/16 Shadow rows. Ordinary rosters/seed 1; no injected state or weakened phase guards." if args.golem_review else None,
        "capture_method": "windowless Bevy arena image-target hook",
        "logical_canvas": CANVAS, "device_scale": 1.0,
        "changed_surfaces": ["one-prism Wisp", "glow and dim-light comparisons", "frozen Ember appearance", "six-button Fort menu", "observer swarm labels"] if args.wisp_review else ["seven-prism stone body", "independent face", "charge/lock/beam", "spherical slam warning", "Fort fifth selector", "observer Golem roster"] if args.golem_review else ["observer mode and rosters", "orbit/free camera", "team body colors", "observer HUD", "terminal results"] if (observer_matrix or args.spectator) else ["map selectors", "authored map terrain and objects", "creature models", "windups", "breath", "barrier", "aura", "party count"] if args.encounter_review else ["charge bar", "release guidance", "partial shield footprint", "ready screen", "paused menu", "actor cameras"] if args.charge_review else ["ready screen", "paused menu", "HUD key guidance"] if args.menu_review else ["terrain", "actor cameras", "cover", "spell effects", "HUD", "tuning", "ready screen"],
        "expected_views": [entry[0] for entry in entries], "mechanical_status": "INCOMPLETE",
        "static_review": "NOT_AN_APPROVAL_PACK" if (args.performance_review or args.spectator_performance) else "UNREVIEWED", "human_motion": "NOT_MEASURED_SYNTHETIC" if args.performance_review else "OBSERVER-CAMERA-MOTION-PENDING" if (observer_matrix or args.spectator) else "HUMAN-MOTION-PENDING",
        "performance_fixture": "Synthetic extra-HP party visits; no ordinary movement or human balance evidence." if args.performance_review else "Ordinary seeded autonomous battle; real app-frame wall intervals, no GPU or vsync measurement." if args.spectator_performance else None,
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
            frame_env.update(battle_environment(row_args, arena_map, matrix=observer_matrix, result=view == "observer-result"))
            if focus:
                frame_env["HEX_ARENA_FOCUS"] = focus
            frame = {
                "name": name, "view": view, "map": arena_map, "encounter": encounter, "terrain_seed": SEEDS[arena_map], "focus_anchor": focus, "started_at": utc_now(), "static_review": "NOT_AN_APPROVAL_PACK" if (args.performance_review or args.spectator_performance) else "UNREVIEWED",
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
    print("Synthetic performance capture complete. No movement, human balance, GPU, or vsync claim." if args.performance_review else "Capture matrix complete. Static review: UNREVIEWED. Native motion: HUMAN-MOTION-PENDING.")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    launch = commands.add_parser("launch", help="Explicitly open the native playable arena through Cargo.")
    captures = commands.add_parser("capture", help="Capture all 23 views without a native window.")
    for command in (launch, captures):
        command.add_argument("--map", choices=MAPS, help="Map recipe (launch: fort; legacy capture: duel).")
        command.add_argument("--encounter", choices=ENCOUNTERS, help="Fort recipe (launch: dragon; legacy capture: shadow).")
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
    review.add_argument("--wisp-review", action="store_true", help="Twelve Wisp body, dim-light, windup, Ember, layered swarm and menu views from ordinary accepted recipes.")
    review.add_argument("--golem-review", action="store_true", help="Twelve natural Fort-player and Duel Golem-vs-Dragon observer body, charge, lock, beam and slam views.")
    review.add_argument("--spectator-review", action="store_true", help="Fourteen Fort/Duel observer menu, whole-map orbit, close two-azimuth, free and terminal views.")
    review.add_argument("--spectator-performance", action="store_true", help="Fort/Duel ordinary observer frame intervals until 3600 ticks or a terminal result; no synthetic HP or movement.")
    review.add_argument("--performance-review", action="store_true", help="Capture five separate synthetic 3600-tick performance fixtures: four Fort presets and all ten Seven Regions enemies.")
    review.add_argument("--encounter-review", action="store_true", help="Capture 27 map, creature, attack-phase, and selector views, including opposite barrier/aura azimuths.")
    review.add_argument("--menu-review", action="store_true",
                        help="Capture the six ready/menu/HUD review views.")
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
        env.update(HEX_ARENA_MAP=args.map or "fort", HEX_ARENA_ENCOUNTER=args.encounter or "dragon")
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
