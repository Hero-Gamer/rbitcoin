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
# Rust regex meta. `<` and `>` are not in this set: `\<` and `\>` are
# word-boundary assertions in the regex crate, not literals.
RUST_META = set(r"\.+*?()|[]{}^$#&-~")
FINISHED = re.compile(r"(?m)^(\d+) mutants tested\b")
OUTCOME = re.compile(r"^(caught|MISSED|TIMEOUT|unviable)\b")


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


def partition(mutants: list[str], changed: set[tuple[str, int]]) -> tuple[list[str], list[str]]:
    new: list[str] = []
    old: list[str] = []
    for line in mutants:
        parsed = parse_mutant(line)
        if parsed and (parsed[0], parsed[1]) in changed:
            new.append(line)
        else:
            old.append(line)
    return new, old


def resume_old(old: list[str], old_index: int, old_name: str) -> tuple[list[str], int]:
    """Rotate ``old`` so the walk starts at ``old_name``, else at ``old_index``.

    A name that is still in the regenerated list wins over the integer. A
    name that is gone uses ``old_index`` modulo the current length, not zero.
    """
    if not old:
        return [], 0
    if old_name:
        for i, line in enumerate(old):
            if line == old_name:
                return old[i:] + old[:i], i
    start = old_index % len(old)
    return old[start:] + old[:start], start


def order(mutants: list[str], changed: set[tuple[str, int]], old_index: int) -> tuple[list[str], list[str]]:
    new, old = partition(mutants, changed)
    rotated, _start = resume_old(old, old_index, "")
    return new, rotated


def next_side(
    elapsed: int,
    budget: int,
    new_remaining: bool,
    old_remaining: bool,
    new_cap_used: bool,
) -> str:
    """Which queue the next file-batch comes from.

    New work goes first until half of this job's budget has elapsed, or
    until this night already used that half. An empty side does not block
    the other. A batch already started is not cancelled here; the caller
    asks again after it finishes, and that next batch is the other side.
    """
    if budget < 1 or elapsed < 0:
        raise ValueError("budget")
    if not new_remaining and not old_remaining:
        return ""
    if new_remaining and not old_remaining:
        return "new"
    if old_remaining and not new_remaining:
        return "old"
    if new_cap_used or elapsed >= budget // 2:
        return "old"
    return "new"


def cap_consumed(elapsed: int, budget: int, new_left: int, old_left: int, already: bool) -> bool:
    """True once this night's new window is spent and backlog work remains.

    An empty side does not burn the window: the other side may use the
    whole job. ``already`` stays set for a later job in the same night so
    that job does not open a second new window.
    """
    if already:
        return True
    if new_left <= 0 or old_left <= 0:
        return False
    if budget < 1 or elapsed < 0:
        raise ValueError("budget")
    return elapsed >= budget // 2


def schedule(
    new: list[str],
    old: list[str],
    budget: int,
    batch_sec: int,
    cap: int,
    new_cap_used: bool,
) -> list[tuple[str, int, str]]:
    """Fake-clock walk used by the queue self-test and the same side rule as the job.

    Each file-batch starts at the current elapsed second and then advances
    the clock by ``batch_sec``. Names in a batch that started before half
    the budget stay on that side.
    """
    if batch_sec < 1:
        raise ValueError("batch-sec")
    new_q = [line.strip() for line in new if parse_mutant(line.strip())]
    old_q = [line.strip() for line in old if parse_mutant(line.strip())]
    elapsed = 0
    used = new_cap_used
    rows: list[tuple[str, int, str]] = []
    while elapsed < budget and (new_q or old_q):
        used = cap_consumed(elapsed, budget, len(new_q), len(old_q), used)
        side = next_side(elapsed, budget, bool(new_q), bool(old_q), used)
        if not side:
            break
        src = new_q if side == "new" else old_q
        batch = file_batch(src, 0, cap)
        if not batch:
            break
        for name in batch:
            rows.append((side, elapsed, name))
        del src[: len(batch)]
        elapsed += batch_sec
    return rows


def begin_night(cursor: dict, run_id: str) -> None:
    """Clear the new-mutant cap when ``run_id`` is a different night.

    An empty ``run_id`` is a local run: the cap starts unused. The same
    id across two jobs keeps a cap the earlier job already consumed.
    """
    if run_id:
        if str(cursor.get("run_id", "")) != run_id:
            cursor["new_cap_used"] = False
            cursor["run_id"] = run_id
    else:
        cursor["new_cap_used"] = False


def read_cursor(path: Path) -> dict:
    return json.loads(path.read_text())


def write_cursor(path: Path, cursor: dict) -> None:
    path.write_text(json.dumps(cursor, indent=2) + "\n")


def rust_re_exact(line: str) -> str:
    """One cargo-mutants ``--list`` line, anchored for ``--re``.

    ``re.escape`` is the wrong escaper: it emits ``\\<`` and ``\\>``, and the
    Rust regex crate treats those as word boundaries.
    """
    body = "".join("\\" + c if c in RUST_META else c for c in line)
    return f"^{body}$"


def file_batch(lines: list[str], offset: int, cap: int) -> list[str]:
    """Queue lines from ``offset`` that share one path, at most ``cap``.

    cargo-mutants 27.1.0 emits ``..`` struct field deletes without applying
    ``--re``. One source file per invocation keeps that repeat inside the
    file under test.
    """
    if offset < 0 or cap < 1:
        raise ValueError("file batch")
    queued = [line.strip() for line in lines if parse_mutant(line.strip())]
    if offset >= len(queued):
        return []
    path = parse_mutant(queued[offset])[0]
    out: list[str] = []
    for line in queued[offset:]:
        if parse_mutant(line)[0] != path or len(out) >= cap:
            break
        out.append(line)
    return out


_OUTCOME_NAME = re.compile(
    r"^(?:caught|MISSED|TIMEOUT|unviable)\s+(.*?)\s+in \d"
)


def prefix_progress(log: str, requested: list[str]) -> int:
    """How far into ``requested`` the log got, in order.

    Outcome lines for other mutants (the unfiltered struct-field deletes)
    do not count, and a later hit does not skip a hole.
    """
    done: set[str] = set()
    for raw in log.splitlines():
        match = _OUTCOME_NAME.match(raw.strip())
        if match:
            done.add(match.group(1))
    count = 0
    for line in requested:
        name = line.strip()
        if not name:
            continue
        if name not in done:
            break
        count += 1
    return count


def finished_mutants(log: str) -> int:
    """How many mutants a cargo-mutants batch actually finished.

    Caught and unviable mutants are not printed as their own lines unless
    ``--caught`` / ``--unviable`` are on. The summary line counts them.
    A batch killed before that line falls back to the outcome lines that
    were printed.
    """
    found = FINISHED.findall(log)
    if found:
        return int(found[-1])
    return sum(1 for line in log.splitlines() if OUTCOME.match(line))


def advance_side(
    cursor: dict,
    side: str,
    completed: int,
    n_new: int,
    n_old: int,
    head: str,
    old_lines: list[str],
    old_offset: int,
    elapsed: int,
    budget: int,
    new_left: int,
    old_left: int,
) -> None:
    """Move one side's cursor and record the next backlog name."""
    if completed < 0:
        raise ValueError("completed")
    if side == "new":
        new_skip = int(cursor.get("new_skip", 0)) + completed
        if new_skip >= n_new:
            cursor["new_base"] = head
            cursor["new_skip"] = 0
        else:
            cursor["new_skip"] = new_skip
    elif side == "old":
        if n_old:
            cursor["old_index"] = (int(cursor.get("old_index", 0)) + completed) % n_old
        else:
            cursor["old_index"] = 0
        queued = [line.strip() for line in old_lines if parse_mutant(line.strip())]
        if queued:
            cursor["old_name"] = queued[old_offset % len(queued)]
        else:
            cursor["old_name"] = ""
    else:
        raise ValueError("side")
    cursor["new_cap_used"] = cap_consumed(
        elapsed,
        budget,
        new_left,
        old_left,
        bool(cursor.get("new_cap_used", False)),
    )


def mark_cap(cursor: dict, elapsed: int, budget: int, new_left: int, old_left: int) -> bool:
    cursor["new_cap_used"] = cap_consumed(
        elapsed,
        budget,
        new_left,
        old_left,
        bool(cursor.get("new_cap_used", False)),
    )
    return bool(cursor["new_cap_used"])


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

    re_p = sub.add_parser("re")
    re_p.add_argument("line")

    fin = sub.add_parser("finished")
    fin.add_argument("--log", type=Path, required=True)

    batch_p = sub.add_parser("file-batch")
    batch_p.add_argument("--queue", type=Path, required=True)
    batch_p.add_argument("--offset", type=int, required=True)
    batch_p.add_argument("--cap", type=int, required=True)

    prog = sub.add_parser("progress")
    prog.add_argument("--log", type=Path, required=True)
    prog.add_argument("--slice", type=Path, required=True)

    sched = sub.add_parser("schedule")
    sched.add_argument("--new", type=Path, required=True)
    sched.add_argument("--old", type=Path, required=True)
    sched.add_argument("--budget", type=int, required=True)
    sched.add_argument("--batch-sec", type=int, required=True)
    sched.add_argument("--cap", type=int, required=True)
    sched.add_argument("--new-cap-used", type=int, default=0)

    resume_p = sub.add_parser("resume-old")
    resume_p.add_argument("--list", type=Path, required=True)
    resume_p.add_argument("--index", type=int, required=True)
    resume_p.add_argument("--name", default="")

    night_p = sub.add_parser("night")
    night_p.add_argument("--cursor", type=Path, required=True)
    night_p.add_argument("--run-id", default="")

    split_p = sub.add_parser("split")
    split_p.add_argument("--list", type=Path, required=True)
    split_p.add_argument("--diff", type=Path, required=True)
    split_p.add_argument("--cursor", type=Path, required=True)
    split_p.add_argument("--out-new", type=Path, required=True)
    split_p.add_argument("--out-old", type=Path, required=True)

    side_p = sub.add_parser("next-side")
    side_p.add_argument("--elapsed", type=int, required=True)
    side_p.add_argument("--budget", type=int, required=True)
    side_p.add_argument("--new-left", type=int, required=True)
    side_p.add_argument("--old-left", type=int, required=True)
    side_p.add_argument("--new-cap-used", type=int, required=True)

    cap_p = sub.add_parser("mark-cap")
    cap_p.add_argument("--cursor", type=Path, required=True)
    cap_p.add_argument("--elapsed", type=int, required=True)
    cap_p.add_argument("--budget", type=int, required=True)
    cap_p.add_argument("--new-left", type=int, required=True)
    cap_p.add_argument("--old-left", type=int, required=True)

    show_p = sub.add_parser("show")
    show_p.add_argument("--cursor", type=Path, required=True)
    show_p.add_argument("--key", required=True)

    adv_side = sub.add_parser("advance-side")
    adv_side.add_argument("--cursor", type=Path, required=True)
    adv_side.add_argument("--side", required=True)
    adv_side.add_argument("--completed", type=int, required=True)
    adv_side.add_argument("--n-new", type=int, required=True)
    adv_side.add_argument("--n-old", type=int, required=True)
    adv_side.add_argument("--head", required=True)
    adv_side.add_argument("--old-queue", type=Path, required=True)
    adv_side.add_argument("--old-offset", type=int, required=True)
    adv_side.add_argument("--elapsed", type=int, required=True)
    adv_side.add_argument("--budget", type=int, required=True)
    adv_side.add_argument("--new-left", type=int, required=True)
    adv_side.add_argument("--old-left", type=int, required=True)

    args = parser.parse_args(argv)
    if args.cmd == "re":
        print(rust_re_exact(args.line))
        return 0
    if args.cmd == "finished":
        print(finished_mutants(args.log.read_text()))
        return 0
    if args.cmd == "file-batch":
        lines = file_batch(args.queue.read_text().splitlines(), args.offset, args.cap)
        for line in lines:
            print(line)
        return 0
    if args.cmd == "progress":
        requested = args.slice.read_text().splitlines()
        print(prefix_progress(args.log.read_text(), requested))
        return 0

    if args.cmd == "order":
        mutants = load_mutants(args.list.read_text())
        changed = changed_lines(args.diff.read_text()) if args.diff.stat().st_size else set()
        new, old = order(mutants, changed, args.old_index)
        if args.new_skip:
            new = new[args.new_skip :]
        for line in new + old:
            print(line)
        return 0

    if args.cmd == "schedule":
        rows = schedule(
            args.new.read_text().splitlines(),
            args.old.read_text().splitlines(),
            args.budget,
            args.batch_sec,
            args.cap,
            bool(args.new_cap_used),
        )
        for side, start, name in rows:
            print(f"{side}\t{start}\t{name}")
        return 0

    if args.cmd == "resume-old":
        old = load_mutants(args.list.read_text())
        rotated, _start = resume_old(old, args.index, args.name)
        for line in rotated:
            print(line)
        return 0

    if args.cmd == "night":
        cursor = read_cursor(args.cursor)
        begin_night(cursor, args.run_id)
        write_cursor(args.cursor, cursor)
        return 0

    if args.cmd == "split":
        mutants = load_mutants(args.list.read_text())
        changed = changed_lines(args.diff.read_text()) if args.diff.stat().st_size else set()
        new, old = partition(mutants, changed)
        # Full counts, before new_skip slices the file the job walks.
        n_new, n_old = len(new), len(old)
        cursor = read_cursor(args.cursor)
        new_skip = int(cursor.get("new_skip", 0))
        if new_skip:
            new = new[new_skip:]
        rotated, start = resume_old(
            old, int(cursor.get("old_index", 0)), str(cursor.get("old_name", ""))
        )
        cursor["old_index"] = start
        cursor["old_name"] = rotated[0] if rotated else ""
        write_cursor(args.cursor, cursor)
        args.out_new.write_text("".join(line + "\n" for line in new))
        args.out_old.write_text("".join(line + "\n" for line in rotated))
        print(n_new, n_old)
        return 0

    if args.cmd == "next-side":
        print(
            next_side(
                args.elapsed,
                args.budget,
                args.new_left > 0,
                args.old_left > 0,
                bool(args.new_cap_used),
            )
        )
        return 0

    if args.cmd == "mark-cap":
        cursor = read_cursor(args.cursor)
        mark_cap(cursor, args.elapsed, args.budget, args.new_left, args.old_left)
        write_cursor(args.cursor, cursor)
        return 0

    if args.cmd == "show":
        cursor = read_cursor(args.cursor)
        key = args.key
        if key == "new_cap_used":
            print("1" if cursor.get("new_cap_used") else "0")
        elif key in ("new_skip", "old_index"):
            print(int(cursor.get(key, 0)))
        else:
            print(cursor.get(key, ""))
        return 0

    if args.cmd == "advance-side":
        cursor = read_cursor(args.cursor)
        old_lines = args.old_queue.read_text().splitlines() if args.old_queue.exists() else []
        advance_side(
            cursor,
            args.side,
            args.completed,
            args.n_new,
            args.n_old,
            args.head,
            old_lines,
            args.old_offset,
            args.elapsed,
            args.budget,
            args.new_left,
            args.old_left,
        )
        write_cursor(args.cursor, cursor)
        return 0

    if args.cmd != "advance":
        raise SystemExit(f"unknown command {args.cmd}")

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
    if old_index != int(cursor.get("old_index", 0)):
        cursor.pop("old_name", None)
    cursor["old_index"] = old_index
    write_cursor(args.cursor, cursor)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
