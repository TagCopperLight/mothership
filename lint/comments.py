#!/usr/bin/env python3
import ast
import io
import subprocess
import sys
import tokenize
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ("e2e/fleet/", "e2e/vm/fleet/", "e2e/debian-packages/")
HASH_FILES = {"ci.sh", "PKGBUILD", ".gitignore"}
HASH_SUFFIXES = {".sh", ".yml", ".yaml", ".Dockerfile"}
UV_BLOCK = ("# /// script", "# ///")


def rust(text: str) -> list[tuple[int, int]]:
    spans, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            end = text.find("\n", i)
            end = n if end < 0 else end
            spans.append((i, end))
            i = end
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while depth and j < n:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            spans.append((i, j))
            i = j
        elif (c == "r" and (raw := raw_string(text, i))) or (
                text.startswith("br", i) and (raw := raw_string(text, i + 1))):
            i = raw
        elif c == '"':
            i = string_end(text, i + 1)
        elif c == "'":
            i = char_end(text, i)
        elif c.isalnum() or c == "_":
            while i < n and (text[i].isalnum() or text[i] == "_"):
                i += 1
        else:
            i += 1
    return spans


def raw_string(text: str, i: int) -> int | None:
    j = i + 1
    while j < len(text) and text[j] == "#":
        j += 1
    if j >= len(text) or text[j] != '"':
        return None
    if i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_") and text[i - 1] != "b":
        return None
    close = '"' + "#" * (j - i - 1)
    end = text.find(close, j + 1)
    return len(text) if end < 0 else end + len(close)


def string_end(text: str, i: int) -> int:
    while i < len(text):
        if text[i] == "\\":
            i += 2
        elif text[i] == '"':
            return i + 1
        else:
            i += 1
    return i


def char_end(text: str, i: int) -> int:
    if text.startswith("\\", i + 1):
        end = text.find("'", i + 3)
        return i + 1 if end < 0 else end + 1
    if i + 2 < len(text) and text[i + 2] == "'":
        return i + 3
    return i + 1


def python(text: str) -> list[tuple[int, int]]:
    offsets = [0]
    for line in text.splitlines(keepends=True):
        offsets.append(offsets[-1] + len(line))
    at = lambda row, col: offsets[row - 1] + col
    spans, in_uv = [], False
    for token in tokenize.generate_tokens(io.StringIO(text).readline):
        if token.type != tokenize.COMMENT:
            continue
        line = token.string.rstrip()
        if token.start == (1, 0) and line.startswith("#!"):
            continue
        if line == UV_BLOCK[0]:
            in_uv = True
        if in_uv:
            in_uv = line != UV_BLOCK[1]
            continue
        spans.append((at(*token.start), at(*token.end)))
    for node in ast.walk(ast.parse(text)):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            first = node.body[0] if node.body else None
            if (isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant)
                    and isinstance(first.value.value, str)):
                spans.append((at(first.lineno, first.col_offset),
                              at(first.end_lineno, first.end_col_offset)))
    return spans


def hashes(text: str) -> list[tuple[int, int]]:
    spans, start = [], 0
    for number, line in enumerate(text.splitlines(keepends=True), 1):
        quote = None
        for col, c in enumerate(line):
            if quote:
                quote = None if c == quote else quote
            elif c in "'\"":
                quote = c
            elif c == "#" and (col == 0 or line[col - 1] in " \t") and not (
                    number == 1 and line.startswith("#!")):
                spans.append((start + col, start + len(line.rstrip("\n"))))
                break
        start += len(line)
    return spans


def finder(path: Path):
    name = path.name
    if path.suffix == ".rs":
        return rust
    if path.suffix == ".py":
        return python
    if name in HASH_FILES or path.suffix in HASH_SUFFIXES or name.endswith(".Dockerfile"):
        return hashes
    first = path.read_text(errors="replace").split("\n", 1)[0]
    if first.startswith("#!") and ("sh" in first.split("/")[-1]):
        return hashes
    return None


def strip(text: str, spans: list[tuple[int, int]]) -> str:
    out, last = [], 0
    for start, end in sorted(spans):
        out.append(text[last:start] + "\n" * text.count("\n", start, end))
        last = end
    out.append(text[last:])
    lines = [after.rstrip() for before, after in zip(text.split("\n"), "".join(out).split("\n"))
             if after.strip() or not before.strip()]
    return "\n".join(lines)


def main() -> int:
    fix = sys.argv[1:] == ["--fix"]
    files = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True,
                           check=True).stdout.split()
    found = 0
    for rel in files:
        path = ROOT / rel
        if rel.startswith(FIXTURES) or not path.is_file():
            continue
        find = finder(path)
        if not find:
            continue
        text = path.read_text()
        spans = find(text)
        if not spans:
            continue
        found += len(spans)
        if fix:
            path.write_text(strip(text, spans))
        else:
            for start, _ in spans:
                print(f"{rel}:{text.count(chr(10), 0, start) + 1}: comment")
    if found and not fix:
        print(f"{found} comments: run lint/comments.py --fix, and put what matters in the docs")
    return 1 if found and not fix else 0


if __name__ == "__main__":
    sys.exit(main())
