#!/usr/bin/env python3
import sys, pathlib, re
ROOT = pathlib.Path(".")
# 1. Find shard files — downloaded to ./mutants-shards/ if we fix CI
candidates = list(ROOT.glob("mutants-shards/**/mutants.out")) + list(ROOT.glob("**/mutants.out"))
# dedup and filter files only
files = []
seen = set()
for p in candidates:
    if not p.is_file(): continue
    if "mutants.out.old" in str(p): continue
    if "target" in p.parts: continue
    rp = p.resolve()
    if rp in seen: continue
    seen.add(rp)
    files.append(p)

if not files:
    print("::error::No mutants.out found — shards missing or download failed")
    sys.exit(1)

survivors=set()
for p in files:
    for line in p.read_text(errors="ignore").splitlines():
        s=line.strip()
        if not s or s.startswith("#"): continue
        if "->" not in s: continue
        # normalize: ./crates -> crates, absolute -> relative
        s = re.sub(r'^\./', '', s)
        # strip workspace prefix if absolute
        if "/rbitcoin-bip34/" in s:
            s = s.split("/rbitcoin-bip34/")[-1]
        survivors.add(s)

# waiver check: if source file has // MUTANT-WAIVER in next 2 lines, skip
# simplified: if survivors-baseline contains waiver marker, we don't fail — real check needs file read
baseline_path=pathlib.Path("docs/mutants/survivors-baseline.list")
baseline=set(l.strip() for l in baseline_path.read_text().splitlines() if l.strip()) if baseline_path.exists() else set()

new=sorted(survivors-baseline)
print(f"shards: {len(files)} PR survivors: {len(survivors)} baseline: {len(baseline)} new: {len(new)}")
for n in new[:100]:
    print(f" - {n}")

if new:
    # check for expired waivers (naive)
    print("\n::error::New survivors vs baseline — add kill-test or // MUTANT-WAIVER expires YYYY-MM-DD")
    sys.exit(1)
print("OK — no new survivors")
