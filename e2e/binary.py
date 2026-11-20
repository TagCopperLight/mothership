import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "target/musl"
BINARY = ROOT / TARGET / "release/mothership"


def build() -> Path:
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    subprocess.run(
        ["docker", "run", "--rm", "--network", "host", "-u", f"{os.getuid()}:{os.getgid()}",
         "-e", "CARGO_HOME=/cargo", "-v", f"{cargo_home}:/cargo", "-v", f"{ROOT}:/src",
         "-w", "/src", "rust:1-alpine",
         "cargo", "build", "--release", "--locked", "--quiet", "--target-dir", TARGET],
        check=True,
    )
    return BINARY
