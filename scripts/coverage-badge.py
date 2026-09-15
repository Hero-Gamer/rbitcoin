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
    base_lh: Optional[int] = None,
    base_lf: Optional[int] = None,
) -> Dict:
    if lf <= 0:
        raise SystemExit("lf must be > 0")
    pct = 100.0 * lh / lf
    message = f"{pct:.2f}%"
    floor_ok = lh * 100 >= lf * gate
    if base_lh is not None and base_lf:
        # Same 2-decimal compare as coverage-gate.passes_ratchet (llvm-cov jitter).
        def hundredths(h: int, f: int) -> int:
            return (10000 * h + f // 2) // f if f > 0 else 0

        ratchet_ok = hundredths(lh, lf) >= hundredths(base_lh, base_lf)
        color = "brightgreen" if floor_ok and ratchet_ok else "red"
    else:
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


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lh", type=int, required=True)
    p.add_argument("--lf", type=int, required=True)
    p.add_argument("--gate", type=int, default=90)
    p.add_argument("--sha", default="")
    p.add_argument("--scope", default="production")
    p.add_argument("--date", default="")
    p.add_argument("--base-lh", type=int, default=None)
    p.add_argument("--base-lf", type=int, default=None)
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
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
