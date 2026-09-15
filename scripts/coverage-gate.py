#!/usr/bin/env python3
"""Pass iff production LCOV LH/LF does not fall vs last green master.

Unrounded integer math: lh/lf >= base_lh/base_lf  ⇔  lh * base_lf >= lf * base_lh.
90% is only a floor when the baseline is missing (offline local) or as a
hard lower bound. GitHub Actions must fetch the baseline (fail closed).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

FLOOR_PCT = 90
DEFAULT_BASELINE_URL = (
    "https://raw.githubusercontent.com/reardencode/rbitcoin/badges/coverage.json"
)
FETCH_TIMEOUT_S = 15


def pct_display(lh: int, lf: int) -> str:
    return f"{100.0 * lh / lf:.2f}"


def passes_floor(lh: int, lf: int, floor: int = FLOOR_PCT) -> bool:
    return lf > 0 and lh * 100 >= lf * floor


def passes_ratchet(lh: int, lf: int, base_lh: int, base_lf: int) -> bool:
    return lf > 0 and base_lf > 0 and lh * base_lf >= lf * base_lh


def load_baseline(src: str) -> Dict[str, Any]:
    if src.startswith("http://") or src.startswith("https://") or src.startswith(
        "file:"
    ):
        req = urllib.request.Request(
            src, headers={"User-Agent": "rbitcoin-coverage-gate"}
        )
        with urllib.request.urlopen(req, timeout=FETCH_TIMEOUT_S) as resp:
            raw = resp.read().decode("utf-8")
        data = json.loads(raw)
    else:
        data = json.loads(Path(src).read_text(encoding="utf-8"))
    lh = int(data["lh"])
    lf = int(data["lf"])
    if lf <= 0:
        raise ValueError(f"baseline lf must be > 0, got {lf}")
    if lh < 0:
        raise ValueError(f"baseline lh must be >= 0, got {lh}")
    return {"lh": lh, "lf": lf, "pct": round(100.0 * lh / lf, 2)}


def resolve_baseline(
    baseline: Optional[str],
    baseline_url: Optional[str],
    floor_only: bool,
) -> Tuple[Optional[Dict[str, Any]], str]:
    """Return (baseline dict or None, mode). mode is ratchet / floor / error."""
    if floor_only:
        return None, "floor"
    src = baseline or os.environ.get("COVERAGE_BASELINE") or ""
    explicit = bool(src)
    if not src:
        src = (
            baseline_url
            or os.environ.get("COVERAGE_BASELINE_URL")
            or DEFAULT_BASELINE_URL
        )
    try:
        return load_baseline(src), "ratchet"
    except (OSError, urllib.error.URLError, ValueError, KeyError, json.JSONDecodeError) as e:
        in_ci = os.environ.get("GITHUB_ACTIONS", "").lower() == "true"
        if in_ci or explicit:
            raise SystemExit(
                f"FAIL: coverage baseline unavailable ({src}): {e}"
            ) from e
        print(
            f"coverage-gate: baseline unavailable ({src}): {e}; using {FLOOR_PCT}% floor",
            file=sys.stderr,
        )
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
    if not passes_ratchet(lh, lf, base_lh, base_lf):
        msg = (
            f"FAIL: line coverage {pct_display(lh, lf)}% ({lh}/{lf}) < "
            f"master {pct_display(base_lh, base_lf)}% ({base_lh}/{base_lf}); "
            f"unrounded LH*base_LF < LF*base_LH"
        )
        return False, msg, status
    status["ok"] = True
    msg = (
        f"Coverage OK: {pct_display(lh, lf)}% ({lh}/{lf}) >= "
        f"master {pct_display(base_lh, base_lf)}% ({base_lh}/{base_lf})"
    )
    return True, msg, status


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lh", type=int, required=True)
    p.add_argument("--lf", type=int, required=True)
    p.add_argument("--baseline", default="", help="Path or URL to badges/coverage.json")
    p.add_argument("--baseline-url", default="", help="Override fetch URL")
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
