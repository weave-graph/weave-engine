#!/usr/bin/env python3
"""Run the Phase A exit scenario (docs/ROADMAP.md) and give one pass or fail.

The scenario is checked by three existing integration tests on the same
generated fleet; this script does not re-implement any of their oracles:

* `weave-hardware/tests/fleet_scenario.rs`: base import, question 1 (every
  update as its own fact write), question 2 against a row oracle, question 5,
  explanations and reproducibility after reopening;
* `weave-hardware/tests/fleet_hybrid.rs`: embeddings, questions 3 and 4
  against an independent ranking, question 5, explanations and reopening;
* `weave-mcp/tests/fleet_exit_questions.rs`: questions 2 to 4 answered by a
  scripted MCP client through the real `weave-mcp` binary over stdio, opened
  on the store after the writer closed it.

Each test prints one `PHASE_A_EXIT {json}` line only after all of its
assertions passed. The script runs each test optimized at the requested size
(default: the exit size, 5,000 units and 1,000 updates), then requires:

* every test passed and printed its line, at the requested size;
* every scenario clause is claimed by the tests expected to check it;
* all three tests generated the same dataset (equal digests) and reached the
  same head revision after the 1,000 updates, so they answered over one fleet
  state, not three similar ones.

It writes a JSON report (`--report`) with per-test durations, the clause map
and the host, and exits non-zero on any failure. Standard library only.
"""
import argparse
import json
import os
import platform
import subprocess
import sys
import time

TESTS = (
    ("weave-hardware", "fleet_scenario"),
    ("weave-hardware", "fleet_hybrid"),
    ("weave-mcp", "fleet_exit_questions"),
)

# Scenario clause -> the (test, check) pairs that must all be reported.
CLAUSES = {
    "fleet: 5,000 units with parts, firmware, sites, owners, series refs and notes": [
        ("fleet_scenario", "import")
    ],
    "fleet: maintenance notes with embeddings": [("fleet_hybrid", "embeddings")],
    "q1: 1,000 incremental corrections and rollouts as individual fact writes": [
        ("fleet_scenario", "q1")
    ],
    "q2: units at site S on firmware F at D as known at K; changes of U between K1 and K2": [
        ("fleet_scenario", "q2")
    ],
    "q3: notes similar to an incident, in the 2-hop neighborhood, valid at the incident": [
        ("fleet_hybrid", "q3")
    ],
    "q4: two similar-unit candidate relations absent from the graph, with provenance": [
        ("fleet_hybrid", "q4")
    ],
    "q5: per-record reader restrictions hide restricted units in every result": [
        ("fleet_scenario", "q5"),
        ("fleet_hybrid", "q5"),
        ("fleet_exit_questions", "q5"),
    ],
    "every answer explains its source facts": [
        ("fleet_scenario", "explain"),
        ("fleet_hybrid", "explain"),
        ("fleet_exit_questions", "explain"),
    ],
    "every answer is reproducible after reopening the database": [
        ("fleet_scenario", "reopen"),
        ("fleet_hybrid", "reopen"),
        ("fleet_exit_questions", "reopen"),
    ],
    "questions 2 to 4 answered by an agent through the A7 MCP preview": [
        ("fleet_exit_questions", "mcp_q2"),
        ("fleet_exit_questions", "mcp_q3"),
        ("fleet_exit_questions", "mcp_q4"),
    ],
}

MARKER = "PHASE_A_EXIT "


def cargo_test(cargo, package, test, release, extra_env):
    command = [cargo, "test", "--locked", "-p", package, "--test", test]
    if release:
        command.append("--release")
    command += ["--", "--nocapture"]
    env = dict(os.environ, **extra_env)
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    markers = []
    for line in process.stdout:
        sys.stdout.write(line)
        if line.startswith(MARKER):
            markers.append(json.loads(line[len(MARKER):]))
    code = process.wait()
    return code, time.monotonic() - started, markers, command


def git_commit():
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--units", type=int, default=5000)
    parser.add_argument("--updates", type=int, default=1000)
    parser.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"))
    parser.add_argument("--debug", action="store_true", help="unoptimized build")
    parser.add_argument("--report", default="phase-a-exit.json")
    args = parser.parse_args()

    extra_env = {
        "WEAVE_FLEET_UNITS": str(args.units),
        "WEAVE_FLEET_UPDATES": str(args.updates),
    }
    failures = []
    tests = {}
    for package, test in TESTS:
        print(f"==> {package} --test {test}", flush=True)
        code, seconds, markers, command = cargo_test(
            args.cargo, package, test, not args.debug, extra_env
        )
        tests[test] = {
            "command": " ".join(command),
            "exit_code": code,
            "seconds": round(seconds, 1),
            "result": markers[-1] if markers else None,
        }
        if code != 0:
            failures.append(f"{test} failed with exit code {code}")
        if len(markers) != 1:
            failures.append(f"{test} printed {len(markers)} PHASE_A_EXIT lines, expected 1")
            continue
        result = markers[0]
        if result.get("test") != test:
            failures.append(f"{test} reported itself as {result.get('test')!r}")
        if (result.get("units"), result.get("updates")) != (args.units, args.updates):
            failures.append(
                f"{test} ran {result.get('units')} units and {result.get('updates')} updates,"
                f" expected {args.units} and {args.updates}"
            )

    reported = {
        (name, check)
        for name, entry in tests.items()
        if entry["result"]
        for check in entry["result"].get("checks", [])
    }
    clauses = {}
    for clause, needed in CLAUSES.items():
        missing = [f"{t}:{c}" for t, c in needed if (t, c) not in reported]
        clauses[clause] = {
            "checked_by": [f"{t}:{c}" for t, c in needed],
            "passed": not missing,
        }
        if missing:
            failures.append(f"clause not checked: {clause} (missing {', '.join(missing)})")

    results = [entry["result"] for entry in tests.values() if entry["result"]]
    digests = {r.get("dataset") for r in results}
    heads = {r.get("head_after_updates") for r in results}
    if len(results) == len(TESTS):
        if len(digests) != 1:
            failures.append(f"the tests generated different datasets: {sorted(map(str, digests))}")
        if len(heads) != 1 or None in heads:
            failures.append(f"the tests reached different heads after the updates: {sorted(map(str, heads))}")

    report = {
        "scenario": "Phase A exit scenario (docs/ROADMAP.md)",
        "passed": not failures,
        "failures": failures,
        "units": args.units,
        "updates": args.updates,
        "optimized": not args.debug,
        "dataset": digests.pop() if len(digests) == 1 else None,
        "head_after_updates": heads.pop() if len(heads) == 1 else None,
        "commit": git_commit(),
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "clauses": clauses,
        "tests": tests,
    }
    with open(args.report, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(f"==> report written to {args.report}")
    for clause, entry in clauses.items():
        print(f"{'PASS' if entry['passed'] else 'FAIL'}  {clause}")
    if failures:
        print("PHASE A EXIT SCENARIO: FAIL")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("PHASE A EXIT SCENARIO: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
