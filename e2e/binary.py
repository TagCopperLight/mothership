import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-unknown-linux-musl"
BINARY = ROOT / "target" / TARGET / "release/mothership"


def build() -> Path:
    subprocess.run(["cargo", "build", "--release", "--locked", "--quiet", "--target", TARGET],
                   cwd=ROOT, check=True)
    return BINARY
