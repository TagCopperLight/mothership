#!/usr/bin/env python3




import difflib
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from functools import cache
from pathlib import Path

import binary

HERE = Path(__file__).resolve().parent
EXIT = re.compile(r"\[exit (\d+)\]")
LAYER_CACHE = os.environ.get("E2E_LAYER_CACHE")


def main() -> int:
    wanted = sys.argv[1:]
    scenarios = sorted(HERE.glob("scenarios/*/*.md"))
    if wanted:
        scenarios = [s for s in scenarios if s.stem in wanted]
    with ThreadPoolExecutor() as pool:
        builds = [pool.submit(binary.build)]
        builds += [pool.submit(build, name) for name in sorted({system(s) for s in scenarios})]
        for done in builds:
            done.result()
    failed = []
    with ThreadPoolExecutor(os.cpu_count()) as pool:
        for path, report in zip(scenarios, pool.map(run, scenarios)):
            name = f"{system(path)}/{path.stem}"
            print(f"{name}: {'ok' if report is None else 'FAIL'}{report or ''}", flush=True)
            if report is not None:
                failed.append(name)
    print(f"\n{len(scenarios) - len(failed)} of {len(scenarios)} scenarios pass")
    for name in failed:
        print(f"  failed: {name}")
    return 1 if failed else 0


def system(scenario: Path) -> str:
    return scenario.parent.name


def image(system: str) -> str:
    return f"mothership-e2e-{system}"


def build(system: str) -> None:
    cache = [f"--cache-from=type={LAYER_CACHE},scope={system}",
             f"--cache-to=type={LAYER_CACHE},scope={system},mode=max"] if LAYER_CACHE else []
    subprocess.run(["docker", "build", "--quiet", "--network", "host", *cache, "-t", image(system),
                    "-f", str(HERE / f"{system}.Dockerfile"), str(HERE)],
                   check=True, stdout=subprocess.DEVNULL)


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


def run(path: Path) -> str | None:
    name = system(path)
    container = subprocess.run(
        ["docker", "run", "-d", "--rm", "--network", "none", "--cap-add", "SYS_ADMIN",
         "--security-opt", "apparmor=unconfined",
         "--hostname", "machine", "--add-host", "machine:127.0.0.1",
         "-v", f"{binary.BINARY}:/usr/local/bin/mothership:ro", image(name), "sleep", "infinity"],
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
                report = ["", "", f"  $ {command}"]
                if out.returncode != want_code:
                    report.append(f"  exit code {out.returncode}, expected {want_code}")
                diff = difflib.unified_diff(expected, actual, "expected", "actual", lineterm="")
                report += [f"  {line}" for line in diff]
                return "\n".join(report) + "\n"
        return None
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
