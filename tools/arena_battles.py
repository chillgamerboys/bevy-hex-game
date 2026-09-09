#!/usr/bin/env python3
"""Run bounded, paired autonomous battles and retain exact-source local receipts."""

from __future__ import annotations

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import shutil
import subprocess
import time

from arena import DEFAULT_TARGET, PRESET_MEMBERS, ROOT, digest, environment, source_state, stop_process, utc_now, write_json


DEFAULT_MATCHUPS = "shadow:dragon,shadow:goblins,shadow:shaman-party,dragon:goblins,dragon:shaman-party,goblins:shaman-party"

def read_rounds(log: Path) -> list[dict]:
    rounds = []
    for line in log.read_text(errors="replace").splitlines():
        if "ARENA_BATTLE " in line:
            rounds.append(json.loads(line.split("ARENA_BATTLE ", 1)[1]))
    return rounds


def validate_rounds(rounds: list[dict], args: argparse.Namespace, *,
                    require_terminal_publication: bool = True) -> None:
    """Admit complete paired receipts; the opt-out is only for historical timing audits."""
    expected = {(seed, a, b, swap)
                for seed in range(args.first_seed, args.first_seed + args.seeds)
                for a, b in (pair.split(":") for pair in args.matchups.split(","))
                for swap in (False, True)}
    found = set()
    for index, row in enumerate(rounds):
        try:
            key = (row["setup"]["seed"], row["first"], row["second"], row["side_and_initiative_swapped"])
            if key not in expected or key in found:
                raise RuntimeError(f"Unexpected or duplicate paired round: {key}")
            _validate_round(row, args, require_terminal_publication)
            found.add(key)
        except (KeyError, TypeError, ValueError, AttributeError, RuntimeError) as error:
            raise RuntimeError(f"Battle receipt row {index}: {error}") from error
    if found != expected:
        raise RuntimeError(f"Missing {len(expected - found)} of {len(expected)} paired rounds.")


def _validate_round(row: dict, args: argparse.Namespace, require_flush: bool) -> None:
    def require(condition: bool, message: str) -> None:
        if not condition:
            raise RuntimeError(message)

    def integer(value: object) -> bool:
        return type(value) is int and value >= 0

    def nonnegative(value: object) -> bool:
        return type(value) in (int, float) and math.isfinite(value) and value >= 0

    setup, summary = row["setup"], row["summary"]
    swap = row["side_and_initiative_swapped"]
    require(type(swap) is bool and integer(setup["seed"]), "Invalid seed or swap type.")
    left, right = (row["second"], row["first"]) if swap else (row["first"], row["second"])
    require((row["left"], row["right"]) == (left, right), "Paired sides were not swapped as requested.")
    require(row["map"] == {"duel": "Duel", "fort": "Fort"}[args.map], "Wrong published map.")
    expected_rosters = [{"team": team, "parties": [PRESET_MEMBERS[preset]]}
                        for team, preset in ((1, left), (2, right))]
    require(setup == {"control": "Spectator", "rosters": expected_rosters,
                      "seed": setup["seed"], "tick_limit": args.seconds * 120, "player_recipe": None},
            "Accepted control, member rosters or tick limit differ from the request.")
    require(all(type(roster["team"]) is int for roster in setup["rosters"]),
            "Accepted roster teams must be integer identities.")
    require(integer(summary["seed"]) and summary["seed"] == setup["seed"], "Summary seed differs from setup.")
    ticks = summary["ticks"]
    require(integer(ticks) and 0 < ticks <= setup["tick_limit"], "Invalid completed tick count.")
    require(nonnegative(summary["seconds"]) and abs(summary["seconds"] - ticks / 120) < 0.0001,
            "Summary seconds do not match completed 120 Hz ticks.")

    actors = row["actors"]
    require(isinstance(actors, list), "Missing actual actor roster.")
    expected_members = Counter((roster["team"], species)
                               for roster in expected_rosters for species in roster["parties"][0])
    require(Counter((a["team"], a["species"]) for a in actors) == expected_members,
            "Actual actor teams/species differ from the accepted roster.")
    require(all(integer(a["id"]) and a["id"] <= 255 and type(a["team"]) is int
                and nonnegative(a["hp"]) for a in actors), "Invalid actor identity, team or HP.")
    require(len({a["id"] for a in actors}) == len(actors), "Duplicate actual actor ID.")
    require(all(integer(s["id"]) for s in row["stats"])
            and Counter((s["id"], s["species"]) for s in row["stats"])
            == Counter((a["id"], a["species"]) for a in actors), "Per-actor statistics omit or duplicate a body.")
    teams = summary["teams"]
    require(isinstance(teams, list) and len(teams) == 2
            and all(type(t["team"]) is int for t in teams)
            and {t["team"] for t in teams} == {1, 2}, "Summary must contain exactly the two accepted teams.")
    living_teams = set()
    for team in teams:
        members = [a for a in actors if a["team"] == team["team"]]
        living = sum(a["hp"] > 0 for a in members)
        require(integer(team["initial"]) and team["initial"] == len(members)
                and integer(team["living"]) and team["living"] == living,
                "Summary initial/living counts differ from actual bodies.")
        require(nonnegative(team["hp"]) and nonnegative(team["max_hp"])
                and team["max_hp"] > 0 and team["hp"] <= team["max_hp"] + 0.001
                and abs(team["hp"] - sum(a["hp"] for a in members)) < 0.001,
                "Summary HP differs from actual bodies or exceeds admitted maximum HP.")
        if living:
            living_teams.add(team["team"])
    result = summary["result"]
    if isinstance(result, dict) and set(result) == {"TeamWinner"}:
        winner = result["TeamWinner"]
        require(type(winner) is int and living_teams == {winner}, "Winner is not the sole living team.")
    elif result == "Draw":
        require(not living_teams, "Elimination draw still has a living team.")
    elif result == "Timeout":
        require(living_teams == {1, 2} and ticks == setup["tick_limit"], "Timeout did not reach the limit with both teams alive.")
    else:
        raise RuntimeError(f"Incomplete, unknown or invalid battle result: {result}")

    if require_flush:
        require(row["setup_source"] == "accepted_battle_setup", "Setup was not read from the accepted session.")
        flush = row["terminal_publication"]
        require(nonnegative(flush["cpu_ms"]) and integer(flush["revision_before"])
                and integer(flush["revision_after"]) and flush["revision_after"] >= flush["revision_before"],
                "Invalid terminal publication timing or revision.")
        require(type(flush["terrain_published"]) is bool
                and flush["terrain_published"] == (flush["revision_after"] != flush["revision_before"]),
                "Terminal publication flag differs from the revision change.")
        require(integer(flush["tick_before"]) and integer(flush["tick_after"])
                and flush["tick_before"] == flush["tick_after"] == ticks
                and flush["battle_state_unchanged"] is True, "Terminal publication advanced or changed the frozen battle.")


def run(args: argparse.Namespace) -> int:
    initial, _, _ = source_state()
    if initial["dirty"]:
        raise RuntimeError("Commit the combined candidate before recording calibration.")
    if not args.output.is_absolute() or args.output.exists():
        raise RuntimeError("--output must name a new absolute directory.")
    if not args.target_dir.is_absolute():
        raise RuntimeError("--target-dir must be absolute.")
    args.output = args.output.resolve()
    args.target_dir = args.target_dir.resolve()
    if not 1 <= args.seeds <= 1000 or not 1 <= args.seconds <= 300:
        raise RuntimeError("Use 1–1000 seeds and 1–300 seconds per round.")
    if not 0 <= args.first_seed <= 2**64 - args.seeds:
        raise RuntimeError("The complete seed range must fit unsigned 64 bits.")
    pairs = [pair.split(":") for pair in args.matchups.split(",")]
    if any(len(pair) != 2 or any(preset not in PRESET_MEMBERS for preset in pair) for pair in pairs) or len({tuple(p) for p in pairs}) != len(pairs):
        raise RuntimeError("Use distinct comma-separated left:right roster pairs.")
    if not 0 < args.timeout < float("inf"):
        raise RuntimeError("--timeout must be finite and positive.")
    if shutil.disk_usage(ROOT).free < 0.75 * 1024**3:
        raise RuntimeError("Less than 0.75 GiB free; preserve the workspace capacity reserve.")
    if args.output.is_relative_to(ROOT) and subprocess.run(
        ("git", "check-ignore", "--quiet", str(args.output)), cwd=ROOT, check=False,
    ).returncode:
        raise RuntimeError("In-repository evidence must be ignored, such as .context/visual-walks/.")
    args.output.mkdir(parents=True)
    env, removed = environment(args.target_dir)
    env.update(HEX_BATTLE_SEED_COUNT=str(args.seeds), HEX_BATTLE_FIRST_SEED=str(args.first_seed),
               HEX_BATTLE_SECONDS=str(args.seconds), HEX_BATTLE_MAP=args.map,
               HEX_BATTLE_MATCHUPS=args.matchups)
    if args.trace:
        env["HEX_BATTLE_TRACE"] = "1"
    features = "dev,arena-prototype,test-support" if args.profile == "dev" else "arena-prototype,test-support"
    command = ["cargo", "test", "-p", "hex_game", "--features", features, "--profile", args.profile,
               "--test", "arena_battles", "calibrate_original_monster_groups", "--", "--ignored",
               "--nocapture", "--test-threads=1"]
    log = args.output / "battles.log"
    receipt = {"source": initial, "started_at": utc_now(), "command": command, "map": args.map,
               "first_seed": args.first_seed, "seeds": args.seeds, "seconds": args.seconds,
               "matchups": args.matchups, "profile": args.profile, "trace": args.trace,
               "target_dir": str(args.target_dir), "removed_capabilities": removed,
               "status": "RUNNING", "rounds": [],
               "configuration_sha256": digest((ROOT / "assets/config/arena.ron").read_bytes()),
               "evidence_boundary": "Actual seeded machine matchups, not human win rates. CI profile or traced timings are diagnostic only; no render, GPU or native-input claim."}
    write_json(args.output / "receipt.json", receipt)
    process = None
    began = time.monotonic()
    print(f"Paired battles: {args.output}", flush=True)
    try:
        with log.open("wb") as output:
            process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=output,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            while process.poll() is None:
                if time.monotonic() - began > args.timeout:
                    raise RuntimeError("Battle command exceeded its explicit wall-time bound.")
                if shutil.disk_usage(ROOT).free < 0.75 * 1024**3:
                    raise RuntimeError("Stopped before consuming the 0.75 GiB capacity reserve.")
                time.sleep(0.5)
        receipt["exit_code"] = process.returncode
        if process.returncode:
            raise RuntimeError(f"Cargo failed with exit {process.returncode}; inspect battles.log.")
        if source_state()[0] != initial:
            raise RuntimeError("Source changed during calibration; retained output is stale.")
        receipt["rounds"] = read_rounds(log)
        configurations = [json.loads(line.split("ARENA_TUNING ", 1)[1])
                          for line in log.read_text(errors="replace").splitlines()
                          if "ARENA_TUNING " in line]
        if len(configurations) != 1:
            raise RuntimeError("Missing or ambiguous accepted configuration record.")
        receipt["configuration"] = configurations[0]
        validate_rounds(receipt["rounds"], args)
        receipt["status"] = "COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        receipt["status"] = "FAILED"
        receipt["error"] = str(error) or "Interrupted"
        raise
    finally:
        if process is not None:
            stop_process(process)
        receipt["finished_at"] = utc_now()
        receipt["wall_seconds"] = time.monotonic() - began
        receipt["log_sha256"] = digest(log.read_bytes()) if log.exists() else None
        write_json(args.output / "receipt.json", receipt)
    print(f"Completed {len(receipt['rounds'])} paired rounds; outcomes retained in receipt.json.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--map", choices=("duel", "fort"), default="duel")
    parser.add_argument("--matchups", default=DEFAULT_MATCHUPS)
    parser.add_argument("--seeds", type=int, default=8)
    parser.add_argument("--first-seed", type=int, default=1)
    parser.add_argument("--seconds", type=int, default=90)
    parser.add_argument("--profile", choices=("dev", "ci"), default="dev")
    parser.add_argument("--target-dir", type=Path, default=DEFAULT_TARGET)
    parser.add_argument("--timeout", type=float, default=2700)
    parser.add_argument("--trace", action="store_true", help="Retain half-second decision diagnostics; timing is then diagnostic only.")
    args = parser.parse_args()
    try:
        return run(args)
    except (RuntimeError, OSError, ValueError) as error:
        parser.exit(1, f"Battle calibration failed: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
