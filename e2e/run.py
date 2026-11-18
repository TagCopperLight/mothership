#!/usr/bin/env python3
"""Runs each scenario in scenarios/ in a fresh container and compares what it prints.

A scenario is a Markdown file. In its ```console blocks, `$ ` starts a command, `> ` continues
it, and the lines up to the next command are what it must print, stdout and stderr together:

- `...` stands for any number of lines, such as pacman's own output.
- A line ending in ` (re)` is a regular expression.
- Other lines match with runs of spaces counted as one, so alignment doesn't matter.
- A last line `[exit N]` means the command must fail with code N. Otherwise it must succeed.

Usage: e2e/run.py [scenario name ...]
"""

import difflib
import re
import subprocess
import sys
from functools import cache
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
IMAGE = "mothership-e2e"
EXIT = re.compile(r"\[exit (\d+)\]")


def main() -> int:
    wanted = sys.argv[1:]
    scenarios = sorted(HERE.glob("scenarios/*.md"))
    if wanted:
        scenarios = [s for s in scenarios if s.stem in wanted]
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    # The host's network, so the build resolves names the way the host does, VPN included.
    subprocess.run(["docker", "build", "--quiet", "--network", "host", "-t", IMAGE, str(HERE)],
                   check=True, stdout=subprocess.DEVNULL)
    failed = [s.stem for s in scenarios if not run(s)]
    print(f"\n{len(scenarios) - len(failed)} of {len(scenarios)} scenarios pass")
    for name in failed:
        print(f"  failed: {name}")
    return 1 if failed else 0


def parse(path: Path) -> list[tuple[str, list[str]]]:
    steps: list[tuple[str, list[str]]] = []
    inside = False
    for line in path.read_text().splitlines():
        if line.startswith("```"):
            inside = line == "```console"
        elif not inside:
            continue
        elif line.startswith("$ "):
            steps.append((line[2:], []))
        elif line.startswith("> ") and steps and not steps[-1][1]:
            command, _ = steps[-1]
            steps[-1] = (command + "\n" + line[2:], [])
        elif steps:
            steps[-1][1].append(line)
    return steps


def run(path: Path) -> bool:
    print(f"{path.stem}: ", end="", flush=True)
    binary = ROOT / "target/debug/mothership"
    container = subprocess.run(
        ["docker", "run", "-d", "--rm", "--network", "none",
         "-v", f"{binary}:/usr/local/bin/mothership:ro", IMAGE, "sleep", "infinity"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    try:
        for command, expected in parse(path):
            out = subprocess.run(
                ["docker", "exec", "-i", container, "bash", "-c", command],
                input="", stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                timeout=600,
            )
            actual = out.stdout.splitlines()
            want_code = 0
            if expected and (m := EXIT.fullmatch(expected[-1].strip())):
                want_code, expected = int(m.group(1)), expected[:-1]
            if out.returncode != want_code or not matches(expected, actual):
                print("FAIL")
                print(f"\n  $ {command}")
                if out.returncode != want_code:
                    print(f"  exit code {out.returncode}, expected {want_code}")
                diff = difflib.unified_diff(expected, actual, "expected", "actual", lineterm="")
                for line in diff:
                    print(f"  {line}")
                print()
                return False
        print("ok")
        return True
    finally:
        subprocess.run(["docker", "rm", "-f", container], capture_output=True)


def matches(expected: list[str], actual: list[str]) -> bool:
    @cache
    def at(e: int, a: int) -> bool:
        if e == len(expected):
            return a == len(actual)
        if expected[e].strip() == "...":
            return any(at(e + 1, rest) for rest in range(a, len(actual) + 1))
        return a < len(actual) and line_matches(expected[e], actual[a]) and at(e + 1, a + 1)

    return at(0, 0)


def line_matches(expected: str, actual: str) -> bool:
    if expected.endswith(" (re)"):
        return re.fullmatch(expected[: -len(" (re)")].strip(), actual.strip()) is not None
    return expected.split() == actual.split()


if __name__ == "__main__":
    sys.exit(main())
