#!/usr/bin/env python3
import sys, pathlib, re
ROOT = pathlib.Path(".")
candidates = list(ROOT.glob("mutants-shards/**/mutants.out")) + list(ROOT.glob("**/mutants.out"))
files, seen = [], set()
for p in candidates:
    if not p.is_file(): continue
    if "mutants.out.old" in str(p): continue
    if "target" in p.parts: continue
    rp = p.resolve()
    if rp in seen: continue
    seen.add(rp)
    files.append(p)

if not files:
    print("::error::No mutants.out found — shards missing")
    sys.exit(1)

survivors=set()
for p in files:
    for line in p.read_text(errors="ignore").splitlines():
        s=line.strip()
        if not s or s.startswith("#"): continue
        if "->" not in s: continue
        s = re.sub(r'^\./', '', s)
        if "/rbitcoin-bip34/" in s:
            s = s.split("/rbitcoin-bip34/")[-1]
        survivors.add(s)

baseline_path=pathlib.Path("docs/mutants/survivors-baseline.list")
baseline=set(l.strip() for l in baseline_path.read_text().splitlines() if l.strip() and not l.strip().startswith("#")) if baseline_path.exists() else set()

# If baseline is empty (first run), create it and pass
if not baseline:
    print(f"Baseline empty — initializing with {len(survivors)} survivors")
    baseline_path.parent.mkdir(parents=True, exist_ok=True)
    baseline_path.write_text("\n".join(sorted(survivors)) + "\n")
    print("OK — baseline initialized")
    sys.exit(0)

new=sorted(survivors-baseline)
print(f"shards: {len(files)} PR survivors: {len(survivors)} baseline: {len(baseline)} new: {len(new)}")
for n in new[:100]:
    print(f" - {n}")

if new:
    print("\n::error::New survivors vs baseline — add kill-test or // MUTANT-WAIVER expires YYYY-MM-DD")
    sys.exit(1)
print("OK — no new survivors")
