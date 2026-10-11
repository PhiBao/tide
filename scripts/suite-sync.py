#!/usr/bin/env python3
"""Prove that the suite in this repository is the suite that actually runs.

The tests live in two places. This repository holds the source of truth -- the
thing a reader reviews, and the thing whose commit history explains why each
assertion exists. TestSprite holds the copy that executes, and it is that copy
which decides whether a pull request goes green.

Nothing kept the two in step, and they duly drifted. A fix landed in the
repository while the portal went on running the old code, and the difference
surfaced only as a CI failure whose reported cause -- "idempotency is not
working" -- was not the real one. Nothing was wrong with idempotency. The old
test was running.

A suite you can read but cannot trust is worse than a smaller suite, because it
invites a reviewer to believe something false. So the binding is explicit
(`tests/suite.json`: file -> test id) and this script checks it in both
directions, for both suites:

  * every file in the repository matches what the portal reports, and
  * every test on the portal is a file in this repository.

A pull request that edits a test without sending it on fails here, before the
suite runs, rather than after a merge and a deploy.

Both layers are authored here and copied to the portal, so both are repaired with
`--push`. `--refresh-plans` is the opposite direction, and exists to re-export
plans the portal regenerated rather than to repair a mistake.

What is checked either way is the same thing: that this repository does not
describe the suite inaccurately. An export that has fallen behind is the same
fault as a fix that never shipped -- it tells a reader something untrue about
what runs.
"""

from __future__ import annotations

import argparse
import difflib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SUITE = ROOT / "tests" / "suite.json"

# TestSprite prepends a small block of framework plumbing to the tests it stores:
# credential placeholders it substitutes at run time. It is the portal's
# business, not ours, so it comes off before comparing. Anything else that
# differs is drift.
INJECTED_HEADER_END = "__AUTH_HEADERS__ = {}"


def cli() -> list[str]:
    """The TestSprite command, however this machine provides it."""
    found = shutil.which("testsprite")
    return [found] if found else ["npx", "--yes", "@testsprite/testsprite-cli"]


def run(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cli() + args, capture_output=True, text=True, timeout=300)


def normalise_code(source: str) -> str:
    """A test's meaningful text, with the portal's plumbing and blank edges removed."""
    marker = source.find(INJECTED_HEADER_END)
    if marker != -1:
        newline = source.find("\n", marker)
        if newline != -1:
            source = source[newline + 1 :]
    lines = [line.rstrip() for line in source.replace("\r\n", "\n").split("\n")]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines) + "\n"


def normalise_plan(steps: object) -> str:
    """A plan compared as data, so formatting in the file cannot matter."""
    return json.dumps(steps, sort_keys=True, separators=(",", ":"))


def portal_tests(project_id: str) -> list[dict]:
    result = run(["test", "list", "--project", project_id, "--output", "json"])
    if result.returncode != 0:
        raise SystemExit(
            "could not list the portal's tests; is TESTSPRITE_API_KEY set?\n"
            + result.stderr.strip()[-800:]
        )
    return json.loads(result.stdout)["items"]


def portal_code(test_id: str) -> str:
    result = run(["test", "code", "get", test_id])
    if result.returncode != 0:
        # transport and auth trouble is reported, never mistaken for drift
        raise SystemExit(
            f"could not read test code for {test_id}:\n" + result.stderr.strip()[-800:]
        )
    return result.stdout


def portal_plan(test_id: str) -> object:
    result = run(["--output", "json", "test", "get", test_id])
    if result.returncode != 0:
        raise SystemExit(
            f"could not read the plan for {test_id}:\n" + result.stderr.strip()[-800:]
        )
    return json.loads(result.stdout).get("planSteps") or []


def code_version(test_id: str) -> str | None:
    result = run(["--output", "json", "test", "code", "get", test_id])
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout).get("codeVersion")
    except json.JSONDecodeError:
        return None


def show(label: str, remote: str, local: str) -> None:
    print(f"\n{'=' * 74}\nDRIFT  {label}\n{'=' * 74}")
    diff = difflib.unified_diff(
        remote.split("\n"),
        local.split("\n"),
        fromfile="what the portal runs",
        tofile="what this repository holds",
        lineterm="",
    )
    for line in list(diff)[:60]:
        print("  " + line)


def check_suite(suite: dict) -> tuple[list[tuple], list[str]]:
    """Compare one suite. Returns (drifted, structural problems)."""
    project_id = suite["project_id"]
    directory = ROOT / suite["dir"]
    bound = {entry["test_id"]: entry for entry in suite["tests"]}
    on_portal = {item["id"]: item for item in portal_tests(project_id)}

    # The UI suite binds its plans and not its code. Frontend tests are executed
    # by an agent against the plan, and the stored code is rewritten by that
    # agent whenever it finds the generated selectors stale -- so the code is a
    # run artefact, not a contract, and committing it would describe something
    # that is not what ran.
    layers = []
    if suite["kind"] in ("plan", "plan+code"):
        layers.append("plan")
    if suite["kind"] in ("code", "plan+code"):
        layers.append("code")

    drifted: list[tuple] = []
    problems: list[str] = []

    for test_id, item in sorted(on_portal.items(), key=lambda kv: kv[1]["name"]):
        entry = bound.get(test_id)
        if entry is None:
            problems.append(
                f"[{suite['key']}] on the portal but not in tests/suite.json: "
                f"{item['name']!r} ({test_id})"
            )
            continue

        for layer in layers:
            relative = entry["file"] if suite["kind"] == "code" else entry.get(layer)
            if not relative:
                problems.append(
                    f"[{suite['key']}] {entry} has no {layer} bound in suite.json"
                )
                continue
            path = directory / relative
            if not path.exists():
                problems.append(
                    f"[{suite['key']}] suite.json names a missing file: {relative}"
                )
                continue

            if layer == "plan":
                stored = json.loads(path.read_text())
                local = normalise_plan(stored.get("planSteps"))
                remote = normalise_plan(portal_plan(test_id))
                if local != remote:
                    drifted.append(
                        (test_id, f"{relative}  <-  {item['name']}", local, remote, "plan")
                    )
            else:
                local = normalise_code(path.read_text())
                remote = normalise_code(portal_code(test_id))
                if local != remote:
                    drifted.append(
                        (test_id, f"{relative}  <-  {item['name']}", local, remote, "code")
                    )

    for test_id, entry in bound.items():
        if test_id not in on_portal:
            label = entry.get("file") or entry.get("code") or entry.get("plan")
            problems.append(f"[{suite['key']}] suite.json names a test the portal lacks: {label}")

    return drifted, problems


def refresh_plans(suite: dict, drifted: list[tuple]) -> int:
    """Write the portal's plan back into this repository's export of it.

    Plans are authored on the portal, so the file here is a copy and this is the
    only direction it can fairly move. The test's id and name are preserved from
    the portal, so a refresh cannot quietly rebind a file to the wrong test.
    """
    directory = ROOT / suite["dir"]
    plans = {e["test_id"]: e["plan"] for e in suite["tests"] if e.get("plan")}
    changed = 0
    for test_id, label, _local, _remote, layer in drifted:
        if layer != "plan":
            continue
        relative = plans.get(test_id)
        if not relative:
            print(f"  SKIP    {label}: no plan file bound")
            continue
        item = next(t for t in portal_tests(suite["project_id"]) if t["id"] == test_id)
        payload = {
            "name": item["name"],
            "test_id": test_id,
            "planSteps": portal_plan(test_id),
        }
        (directory / relative).write_text(json.dumps(payload, indent=2) + "\n")
        print(f"  refreshed  {relative}")
        changed += 1
    return changed


def push(suite: dict, drifted: list[tuple]) -> int:
    """Send this repository's version of each drifted artefact to the portal.

    Both layers go this way. The plan is pushed with `test plan put --steps`,
    which takes the same shape the file here already has, so the file that a
    reader reviews is the file that is sent -- nothing is translated on the way
    out, and there is no second format to keep in step.
    """
    directory = ROOT / suite["dir"]
    failures = 0
    for test_id, label, _local, _remote, layer in drifted:
        entry = next((e for e in suite["tests"] if e["test_id"] == test_id), None)
        if entry is None:
            failures += 1
            print(f"  SKIP    {label}: not bound in tests/suite.json")
            continue

        if layer == "plan":
            relative = entry.get("plan")
            if not relative:
                failures += 1
                print(f"  SKIP    {label}: no plan file bound")
                continue
            result = run(
                ["test", "plan", "put", test_id, "--steps", str(directory / relative)]
            )
        else:
            relative = entry.get("code") or entry.get("file")
            if not relative:
                failures += 1
                print(f"  SKIP    {label}: no code file bound")
                continue
            version = code_version(test_id)
            args = ["test", "code", "put", test_id, "--code-file", str(directory / relative)]
            args += ["--expected-version", version] if version else ["--force"]
            result = run(args)

        if result.returncode == 0:
            print(f"  pushed  {relative}")
        else:
            failures += 1
            print(f"  FAILED  {relative}: {result.stderr.strip()[-200:]}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--push",
        action="store_true",
        help="send this repository's version of each drifted test to the portal",
    )
    parser.add_argument(
        "--refresh-plans",
        action="store_true",
        help="rewrite this repository's plan exports from the portal",
    )
    args = parser.parse_args()

    suite_file = json.loads(SUITE.read_text())
    all_drifted: list[tuple] = []
    all_problems: list[str] = []
    totals: list[str] = []

    for suite in suite_file["suites"]:
        drifted, problems = check_suite(suite)
        all_drifted += [(suite, *row) for row in drifted]
        all_problems += problems
        totals.append(f"{len(suite['tests'])} {suite['key']}")

    for _suite, _test_id, label, local, remote, _layer in all_drifted:
        show(label, remote, local)

    if all_problems:
        print("\nSTRUCTURE")
        for problem in all_problems:
            print(f"  - {problem}")

    if not all_drifted and not all_problems:
        print(f"in sync: {', '.join(totals)} tests, all bound, no drift")
        return 0

    if all_drifted and (args.push or args.refresh_plans):
        failures = 0
        if args.push:
            print("\nPUSHING this repository's version")
            for suite in suite_file["suites"]:
                rows = [row for row in all_drifted if row[0] is suite]
                if rows:
                    print(f"  {suite['key']}:")
                    failures += push(suite, [row[1:] for row in rows])
        if args.refresh_plans:
            print("\nREFRESHING this repository's plan exports from the portal")
            for suite in suite_file["suites"]:
                rows = [
                    row
                    for row in all_drifted
                    if row[0] is suite and row[5] == "plan" and not args.push
                ]
                if rows:
                    print(f"  {suite['key']}:")
                    refresh_plans(suite, [row[1:] for row in rows])
        if failures == 0 and not all_problems:
            print("\nnow re-run to confirm the two agree")
            return 0
        return 1

    if all_drifted and not args.push and not args.refresh_plans:
        print(
            f"\n{len(all_drifted)} artefact(s) drifted. The portal runs a different "
            "suite from the one this repository describes, so a green run would not "
            "be evidence about the code under review.\n\nFix with:\n\n"
            "    python3 scripts/suite-sync.py --push\n"
        )
    return 1


if __name__ == "__main__":
    sys.exit(main())
