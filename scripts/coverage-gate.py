#!/usr/bin/env python3
"""Pass iff production LCOV LH/LF does not fall vs master at the PR fork.

Baseline is the **highest** published master coverage job whose SHA is an
ancestor of `git merge-base(HEAD, origin/master)` (the fork point). Master
jobs that landed after the PR branched are ignored so the target does not
move while the PR is open. Unrounded: lh/lf >= base_lh/base_lf  ⇔
lh * base_lf >= lf * base_lh. 90% is only a floor when that snapshot is
missing (offline local). GitHub Actions must fetch history (fail closed).
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

FLOOR_PCT = 90
DEFAULT_BASELINE_URL = (
    "https://raw.githubusercontent.com/reardencode/rbitcoin/badges/coverage.json"
)
DEFAULT_HISTORY_URL = (
    "https://raw.githubusercontent.com/reardencode/rbitcoin/badges/coverage-history.jsonl"
)
FETCH_TIMEOUT_S = 15


def pct_display(lh: int, lf: int) -> str:
    return f"{100.0 * lh / lf:.2f}"


def passes_floor(lh: int, lf: int, floor: int = FLOOR_PCT) -> bool:
    return lf > 0 and lh * 100 >= lf * floor


def passes_ratchet(lh: int, lf: int, base_lh: int, base_lf: int) -> bool:
    return lf > 0 and base_lf > 0 and lh * base_lf >= lf * base_lh


def _fetch_text(src: str) -> str:
    if src.startswith("http://") or src.startswith("https://") or src.startswith(
        "file:"
    ):
        req = urllib.request.Request(
            src, headers={"User-Agent": "rbitcoin-coverage-gate"}
        )
        with urllib.request.urlopen(req, timeout=FETCH_TIMEOUT_S) as resp:
            return resp.read().decode("utf-8")
    return Path(src).read_text(encoding="utf-8")


def load_baseline(src: str) -> Dict[str, Any]:
    data = json.loads(_fetch_text(src))
    lh = int(data["lh"])
    lf = int(data["lf"])
    if lf <= 0:
        raise ValueError(f"baseline lf must be > 0, got {lf}")
    if lh < 0:
        raise ValueError(f"baseline lh must be >= 0, got {lh}")
    out = {"lh": lh, "lf": lf, "pct": round(100.0 * lh / lf, 2)}
    sha = data.get("sha")
    if sha:
        out["sha"] = str(sha)
    date = data.get("date")
    if date:
        out["date"] = str(date)
    return out


def parse_history(text: str) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        data = json.loads(line)
        lh = int(data["lh"])
        lf = int(data["lf"])
        sha = str(data["sha"])
        if lf <= 0 or not sha:
            continue
        rows.append(
            {
                "sha": sha,
                "lh": lh,
                "lf": lf,
                "pct": round(100.0 * lh / lf, 2),
                "date": str(data.get("date") or ""),
            }
        )
    return rows


def is_ancestor(maybe_anc: str, desc: str, git_dir: str) -> bool:
    r = subprocess.run(
        ["git", "-C", git_dir, "merge-base", "--is-ancestor", maybe_anc, desc],
        capture_output=True,
        check=False,
    )
    return r.returncode == 0


def _ratio_better(a: Dict[str, Any], b: Dict[str, Any]) -> bool:
    """True if a's LH/LF is higher than b's (then higher LH, then later date)."""
    cross = a["lh"] * b["lf"] - b["lh"] * a["lf"]
    if cross != 0:
        return cross > 0
    if a["lh"] != b["lh"]:
        return a["lh"] > b["lh"]
    return str(a.get("date") or "") > str(b.get("date") or "")


def pick_baseline(
    entries: List[Dict[str, Any]], merge_base: str, git_dir: str
) -> Optional[Dict[str, Any]]:
    """Highest coverage among jobs whose SHA is an ancestor of merge_base."""
    best: Optional[Dict[str, Any]] = None
    for e in entries:
        if not is_ancestor(str(e["sha"]), merge_base, git_dir):
            continue
        if best is None or _ratio_better(e, best):
            best = e
    return best


def resolve_baseline(
    baseline: Optional[str],
    baseline_url: Optional[str],
    floor_only: bool,
    history: Optional[str] = None,
    history_url: Optional[str] = None,
    merge_base: Optional[str] = None,
    git_dir: Optional[str] = None,
) -> Tuple[Optional[Dict[str, Any]], str]:
    """Return (baseline dict or None, mode). mode is ratchet / floor / error."""
    if floor_only:
        return None, "floor"
    in_ci = os.environ.get("GITHUB_ACTIONS", "").lower() == "true"
    git_dir = git_dir or os.environ.get("COVERAGE_GIT_DIR") or "."
    merge_base = merge_base or os.environ.get("COVERAGE_MERGE_BASE") or ""

    hist_src = history or os.environ.get("COVERAGE_HISTORY") or ""
    hist_explicit = bool(hist_src)
    if not hist_src:
        hist_src = (
            history_url
            or os.environ.get("COVERAGE_HISTORY_URL")
            or DEFAULT_HISTORY_URL
        )
    entries: List[Dict[str, Any]] = []
    try:
        entries = parse_history(_fetch_text(hist_src))
    except (OSError, urllib.error.URLError, ValueError, KeyError, json.JSONDecodeError) as e:
        if hist_explicit and in_ci:
            raise SystemExit(
                f"FAIL: coverage history unavailable ({hist_src}): {e}"
            ) from e
        print(
            f"coverage-gate: history unavailable ({hist_src}): {e}",
            file=sys.stderr,
        )

    src = baseline or os.environ.get("COVERAGE_BASELINE") or ""
    explicit = bool(src)
    if not src:
        src = (
            baseline_url
            or os.environ.get("COVERAGE_BASELINE_URL")
            or DEFAULT_BASELINE_URL
        )
    current: Optional[Dict[str, Any]] = None
    try:
        current = load_baseline(src)
    except (OSError, urllib.error.URLError, ValueError, KeyError, json.JSONDecodeError) as e:
        if (in_ci or explicit) and not entries:
            raise SystemExit(
                f"FAIL: coverage baseline unavailable ({src}): {e}"
            ) from e
        if not entries:
            print(
                f"coverage-gate: baseline unavailable ({src}): {e}; using {FLOOR_PCT}% floor",
                file=sys.stderr,
            )
            return None, "floor"

    if current is not None:
        sha = str(current.get("sha") or "")
        if sha and not any(str(e.get("sha")) == sha for e in entries):
            entries.append(current)

    if merge_base:
        picked = pick_baseline(entries, merge_base, git_dir)
        if picked is not None:
            return picked, "ratchet"
        if current is not None:
            print(
                f"coverage-gate: no snapshot at or before {merge_base}; "
                "using latest badge (history still seeding)",
                file=sys.stderr,
            )
            return current, "ratchet"
        if in_ci:
            raise SystemExit(
                f"FAIL: no coverage snapshot at or before merge-base {merge_base}"
            )
        print(
            f"coverage-gate: no snapshot at or before {merge_base}; using {FLOOR_PCT}% floor",
            file=sys.stderr,
        )
        return None, "floor"

    if current is not None:
        return current, "ratchet"
    if in_ci:
        raise SystemExit("FAIL: coverage baseline unavailable (no merge-base, no badge)")
    return None, "floor"


def decide(
    lh: int, lf: int, base: Optional[Dict[str, Any]]
) -> Tuple[bool, str, Dict[str, Any]]:
    if lf <= 0:
        status = {
            "ok": False,
            "lh": lh,
            "lf": lf,
            "pct": 0.0,
            "floor": FLOOR_PCT,
            "mode": "error",
        }
        return False, "FAIL: no LCOV totals (lf <= 0)", status
    pct = round(100.0 * lh / lf, 2)
    status: Dict[str, Any] = {
        "ok": False,
        "lh": lh,
        "lf": lf,
        "pct": pct,
        "floor": FLOOR_PCT,
        "mode": "floor" if base is None else "ratchet",
    }
    if not passes_floor(lh, lf):
        msg = (
            f"FAIL: line coverage {pct_display(lh, lf)}% < {FLOOR_PCT}% "
            f"({lh}/{lf}; unrounded LH*100 < LF*{FLOOR_PCT})"
        )
        return False, msg, status
    if base is None:
        status["ok"] = True
        msg = (
            f"Coverage OK: {pct_display(lh, lf)}% ≥ {FLOOR_PCT}% floor "
            f"({lh}/{lf}; no master baseline)"
        )
        return True, msg, status
    base_lh = int(base["lh"])
    base_lf = int(base["lf"])
    status["base_lh"] = base_lh
    status["base_lf"] = base_lf
    status["base_pct"] = round(100.0 * base_lh / base_lf, 2)
    if base.get("sha"):
        status["base_sha"] = base["sha"]
    label = "merge-base" if base.get("sha") else "master"
    sha_note = f"; sha {base['sha']}" if base.get("sha") else ""
    if not passes_ratchet(lh, lf, base_lh, base_lf):
        msg = (
            f"FAIL: line coverage {pct_display(lh, lf)}% ({lh}/{lf}) < "
            f"{label} {pct_display(base_lh, base_lf)}% ({base_lh}/{base_lf}{sha_note}); "
            f"unrounded LH*base_LF < LF*base_LH"
        )
        return False, msg, status
    status["ok"] = True
    msg = (
        f"Coverage OK: {pct_display(lh, lf)}% ({lh}/{lf}) >= "
        f"{label} {pct_display(base_lh, base_lf)}% ({base_lh}/{base_lf}{sha_note})"
    )
    return True, msg, status


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lh", type=int, required=True)
    p.add_argument("--lf", type=int, required=True)
    p.add_argument("--baseline", default="", help="Path or URL to badges/coverage.json")
    p.add_argument("--baseline-url", default="", help="Override fetch URL")
    p.add_argument(
        "--history",
        default="",
        help="Path or URL to badges/coverage-history.jsonl",
    )
    p.add_argument("--history-url", default="", help="Override history fetch URL")
    p.add_argument(
        "--merge-base",
        default="",
        help="git merge-base of the PR tip and origin/master",
    )
    p.add_argument(
        "--git-dir",
        default="",
        help="Repo for ancestor checks (default: cwd)",
    )
    p.add_argument(
        "--floor-only",
        action="store_true",
        help="Skip ratchet; 90% floor only",
    )
    p.add_argument("--status-out", default="", help="Write decision JSON")
    args = p.parse_args()
    try:
        base, _mode = resolve_baseline(
            args.baseline or None,
            args.baseline_url or None,
            args.floor_only,
            args.history or None,
            args.history_url or None,
            args.merge_base or None,
            args.git_dir or None,
        )
    except SystemExit:
        raise
    ok, msg, status = decide(args.lh, args.lf, base)
    if args.status_out:
        Path(args.status_out).write_text(
            json.dumps(status, indent=2) + "\n", encoding="utf-8"
        )
    if ok:
        print(msg)
        sys.exit(0)
    print(msg, file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
