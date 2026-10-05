#!/usr/bin/env python3
"""Write Shields endpoint JSON for a coverage measurement.

Optional --lcov is the filtered production report from coverage.sh. Crate
rows are that file grouped by crates/<name>/, and they must sum to --lh/--lf.
"""
import argparse
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional, Tuple

_CRATE = re.compile(r"crates/([^/]+)/")


def badge_payload(
    lh: int,
    lf: int,
    gate: int,
    sha: str,
    scope: str,
    date: Optional[str] = None,
    base_lh: Optional[int] = None,
    base_lf: Optional[int] = None,
) -> Dict:
    if lf <= 0:
        raise SystemExit("lf must be > 0")
    pct = 100.0 * lh / lf
    message = f"{pct:.2f}%"
    floor_ok = lh * 100 >= lf * gate
    color = "brightgreen" if floor_ok else "red"
    out = {
        "schemaVersion": 1,
        "label": "coverage",
        "message": message,
        "color": color,
        "pct": round(pct, 2),
        "lh": lh,
        "lf": lf,
        "gate": gate,
        "scope": scope,
        "sha": sha[:12],
        "date": date or datetime.now(timezone.utc).strftime("%Y-%m-%d"),
    }
    if base_lh is not None and base_lf:
        out["base_lh"] = base_lh
        out["base_lf"] = base_lf
        out["base_pct"] = round(100.0 * base_lh / base_lf, 2)
    return out


def production_crates(text: str) -> Tuple[List[Dict], int, int]:
    """Group a filtered LCOV report by crate. Lines outside crates/ fail."""
    totals: Dict[str, List[int]] = {}
    other = 0
    current: Optional[str] = None
    lh: Optional[int] = None
    lf: Optional[int] = None

    def finish() -> None:
        nonlocal current, lh, lf, other
        if current is None:
            return
        if lh is None and lf is None:
            current = None
            return
        if lh is None or lf is None or lf < 0 or lh < 0 or lh > lf:
            raise SystemExit(f"lcov record out of range: {current} lh={lh} lf={lf}")
        match = _CRATE.search(current)
        if match is None:
            other += lf
        else:
            slot = totals.setdefault(match.group(1), [0, 0])
            slot[0] += lh
            slot[1] += lf
        current = None
        lh = None
        lf = None

    for line in text.splitlines():
        if line.startswith("SF:"):
            finish()
            current = line[3:]
        elif line.startswith("LH:"):
            lh = int(line[3:])
        elif line.startswith("LF:"):
            lf = int(line[3:])
        elif line == "end_of_record":
            finish()
    finish()
    if other:
        raise SystemExit(f"lcov has {other} lines outside crates/")
    rows: List[Dict] = []
    sum_lh = 0
    sum_lf = 0
    for name in sorted(totals):
        hit, found = totals[name]
        if found <= 0:
            continue
        sum_lh += hit
        sum_lf += found
        rows.append(
            {
                "name": name,
                "lh": hit,
                "lf": found,
                "pct": round(100.0 * hit / found, 2),
            }
        )
    return rows, sum_lh, sum_lf


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lh", type=int, required=True)
    p.add_argument("--lf", type=int, required=True)
    p.add_argument("--gate", type=int, default=92)
    p.add_argument("--sha", default="")
    p.add_argument("--scope", default="production")
    p.add_argument("--date", default="")
    p.add_argument("--base-lh", type=int, default=None)
    p.add_argument("--base-lf", type=int, default=None)
    p.add_argument("--lcov", default="", help="filtered production LCOV; adds crates")
    p.add_argument("--out", required=True)
    args = p.parse_args()
    payload = badge_payload(
        args.lh,
        args.lf,
        args.gate,
        args.sha,
        args.scope,
        args.date or None,
        args.base_lh,
        args.base_lf,
    )
    if args.lcov:
        rows, sum_lh, sum_lf = production_crates(
            Path(args.lcov).read_text(encoding="utf-8", errors="replace")
        )
        if sum_lh != args.lh or sum_lf != args.lf:
            raise SystemExit(
                f"crate LCOV {sum_lh}/{sum_lf} != badge {args.lh}/{args.lf}"
            )
        payload["crates"] = rows
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
