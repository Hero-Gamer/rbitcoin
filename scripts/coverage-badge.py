#!/usr/bin/env python3
"""Write Shields endpoint JSON for a coverage measurement."""
import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, Optional


def badge_payload(
    lh: int,
    lf: int,
    gate: int,
    sha: str,
    scope: str,
    date: Optional[str] = None,
) -> Dict:
    if lf <= 0:
        raise SystemExit("lf must be > 0")
    pct = 100.0 * lh / lf
    message = f"{pct:.2f}%"
    color = "brightgreen" if lh * 100 >= lf * gate else "red"
    return {
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


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lh", type=int, required=True)
    p.add_argument("--lf", type=int, required=True)
    p.add_argument("--gate", type=int, default=90)
    p.add_argument("--sha", default="")
    p.add_argument("--scope", default="production")
    p.add_argument("--date", default="")
    p.add_argument("--out", required=True)
    args = p.parse_args()
    payload = badge_payload(
        args.lh,
        args.lf,
        args.gate,
        args.sha,
        args.scope,
        args.date or None,
    )
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
