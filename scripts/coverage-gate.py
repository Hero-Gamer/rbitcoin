#!/usr/bin/env python3
"""Pass iff production LCOV is at least FLOOR_PCT (unrounded LH*100 >= LF*floor).

llvm-cov hit counts jitter tens of lines on the same tree; a never-falls
ratchet against master is not a stable gate. 92% is a fixed floor.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, Tuple

FLOOR_PCT = 92


def pct_display(lh: int, lf: int) -> str:
    if lf <= 0:
        return "0.00"
    return f"{(10000 * lh + lf // 2) // lf / 100:.2f}"


def passes_floor(lh: int, lf: int, floor: int = FLOOR_PCT) -> bool:
    return lf > 0 and lh * 100 >= lf * floor


def decide(lh: int, lf: int) -> Tuple[bool, str, Dict[str, Any]]:
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
        "mode": "floor",
    }
    if not passes_floor(lh, lf):
        msg = (
            f"FAIL: line coverage {pct_display(lh, lf)}% < {FLOOR_PCT}% "
            f"({lh}/{lf}; unrounded LH*100 < LF*{FLOOR_PCT})"
        )
        return False, msg, status
    status["ok"] = True
    msg = (
        f"Coverage OK: {pct_display(lh, lf)}% ≥ {FLOOR_PCT}% floor ({lh}/{lf})"
    )
    return True, msg, status


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lh", type=int, required=True)
    p.add_argument("--lf", type=int, required=True)
    p.add_argument("--status-out", default="", help="Write decision JSON")
    # Ignored leftovers so old wrappers do not break.
    p.add_argument("--baseline", default="")
    p.add_argument("--baseline-url", default="")
    p.add_argument("--history", default="")
    p.add_argument("--history-url", default="")
    p.add_argument("--merge-base", default="")
    p.add_argument("--git-dir", default="")
    p.add_argument("--floor-only", action="store_true")
    args = p.parse_args()
    ok, msg, status = decide(args.lh, args.lf)
    if args.status_out:
        Path(args.status_out).write_text(
            json.dumps(status, indent=2) + "\n", encoding="utf-8"
        )
    print(msg)
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
