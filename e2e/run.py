#!/usr/bin/env python3




import difflib
import os
import re
import subprocess
import sys
from functools import cache
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
EXIT = re.compile(r"\[exit (\d+)\]")
BINARIES = {
    "arch": ROOT / "target/debug/mothership",
    "debian": ROOT / "target/debian/debug/mothership",
}


def main() -> int:
    wanted = sys.argv[1:]
    scenarios = sorted(HERE.glob("scenarios/*/*.md"))
    if wanted:
        scenarios = [s for s in scenarios if s.stem in wanted]
    systems = sorted({system(s) for s in scenarios})
    for name in systems:
        build(name)
    failed = [f"{system(s)}/{s.stem}" for s in scenarios if not run(s)]
    print(f"\n{len(scenarios) - len(failed)} of {len(scenarios)} scenarios pass")
    for name in failed:
        print(f"  failed: {name}")
    return 1 if failed else 0


def system(scenario: Path) -> str:
    return scenario.parent.name


def image(system: str) -> str:
    return f"mothership-e2e-{system}"


def build(system: str) -> None:
    if system == "debian":
        cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
        subprocess.run(
            ["docker", "run", "--rm", "--network", "host", "-u", f"{os.getuid()}:{os.getgid()}",
             "-e", "CARGO_HOME=/cargo", "-v", f"{cargo_home}:/cargo", "-v", f"{ROOT}:/src",
             "-w", "/src", "rust:1-bookworm",
             "cargo", "build", "--locked", "--quiet", "--target-dir", "target/debian"],
            check=True,
        )
    else:
        subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    subprocess.run(["docker", "build", "--quiet", "--network", "host", "-t", image(system),
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


def run(path: Path) -> bool:
    name = system(path)
    print(f"{name}/{path.stem}: ", end="", flush=True)
    container = subprocess.run(
        ["docker", "run", "-d", "--rm", "--network", "none",
         "--hostname", "machine", "--add-host", "machine:127.0.0.1",
         "-v", f"{BINARIES[name]}:/usr/local/bin/mothership:ro", image(name), "sleep", "infinity"],
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
