#!/usr/bin/env python3
import sys, pathlib
survivors=set()
for p in pathlib.Path(".").rglob("mutants.out"):
    if not p.is_file():
        continue
    # skip old archives
    if "mutants.out.old" in str(p):
        continue
    for line in p.read_text(errors="ignore").splitlines():
        s=line.strip()
        if s and not s.startswith("#") and "->" in s:
            survivors.add(s)

baseline_path=pathlib.Path("docs/mutants/survivors-baseline.list")
baseline=set(l.strip() for l in baseline_path.read_text().splitlines() if l.strip()) if baseline_path.exists() else set()

new=sorted(survivors-baseline)
print(f"PR survivors: {len(survivors)} baseline: {len(baseline)} new: {len(new)}")
for n in new[:100]:
    print(f" - {n}")
if new:
    print("\n::error::New survivors vs baseline — add kill-test or // MUTANT-WAIVER")
    sys.exit(1)
print("OK — no new survivors")
