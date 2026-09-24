#!/usr/bin/env python3
"""Order cargo-mutants names: changed lines first, then a rotating backlog.

The nightly job runs a time budget of these names with ``--test-workspace``.
A cursor remembers how far the new-code prefix and the old rotation got.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def parse_mutant(line: str) -> tuple[str, int, str] | None:
    parts = line.split(":", 3)
    if len(parts) < 4:
        return None
    path, line_no, _col, _rest = parts
    try:
        return path, int(line_no), line
    except ValueError:
        return None


def changed_lines(diff_text: str) -> set[tuple[str, int]]:
    """New-file line numbers touched by a ``git diff -U0``."""
    path = ""
    new_line = 0
    touched: set[tuple[str, int]] = set()
    for raw in diff_text.splitlines():
        if raw.startswith("+++ b/"):
            path = raw[6:]
            continue
        if raw.startswith("+++ /dev/null"):
            path = ""
            continue
        match = HUNK.match(raw)
        if match:
            new_line = int(match.group(1))
            continue
        if not path or raw.startswith(("diff ", "index ", "--- ")):
            continue
        if raw.startswith("+"):
            touched.add((path, new_line))
            new_line += 1
        elif raw.startswith("-"):
            continue
        elif raw.startswith("\\"):
            continue
        else:
            new_line += 1
    return touched


def load_mutants(text: str) -> list[str]:
    out = []
    for raw in text.splitlines():
        line = raw.strip()
        if parse_mutant(line):
            out.append(line)
    return out


def order(mutants: list[str], changed: set[tuple[str, int]], old_index: int) -> tuple[list[str], list[str]]:
    new: list[str] = []
    old: list[str] = []
    for line in mutants:
        parsed = parse_mutant(line)
        if parsed and (parsed[0], parsed[1]) in changed:
            new.append(line)
        else:
            old.append(line)
    if old:
        start = old_index % len(old)
        old = old[start:] + old[:start]
    return new, old


def advance(new_skip: int, old_index: int, n_new: int, n_old: int, completed: int) -> tuple[int, int, bool]:
    """Return (new_skip, old_index, new_done).

    ``new_done`` means every new mutant was attempted, so the caller can
    move ``new_base`` to HEAD.
    """
    if completed < 0:
        raise ValueError("completed")
    take_new = min(completed, max(n_new - new_skip, 0))
    new_skip += take_new
    rest = completed - take_new
    if n_old and rest:
        old_index = (old_index + rest) % n_old
    elif n_old == 0:
        old_index = 0
    return new_skip, old_index, new_skip >= n_new


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    order_p = sub.add_parser("order")
    order_p.add_argument("--list", type=Path, required=True)
    order_p.add_argument("--diff", type=Path, required=True)
    order_p.add_argument("--old-index", type=int, required=True)
    order_p.add_argument("--new-skip", type=int, default=0)

    adv = sub.add_parser("advance")
    adv.add_argument("--cursor", type=Path, required=True)
    adv.add_argument("--n-new", type=int, required=True)
    adv.add_argument("--n-old", type=int, required=True)
    adv.add_argument("--completed", type=int, required=True)
    adv.add_argument("--head", required=True)

    args = parser.parse_args(argv)
    if args.cmd == "order":
        mutants = load_mutants(args.list.read_text())
        changed = changed_lines(args.diff.read_text()) if args.diff.stat().st_size else set()
        new, old = order(mutants, changed, args.old_index)
        if args.new_skip:
            new = new[args.new_skip :]
        for line in new + old:
            print(line)
        return 0

    cursor = json.loads(args.cursor.read_text())
    new_skip, old_index, done = advance(
        int(cursor.get("new_skip", 0)),
        int(cursor.get("old_index", 0)),
        args.n_new,
        args.n_old,
        args.completed,
    )
    if done:
        cursor["new_base"] = args.head
        cursor["new_skip"] = 0
    else:
        cursor["new_skip"] = new_skip
    cursor["old_index"] = old_index
    args.cursor.write_text(json.dumps(cursor, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
