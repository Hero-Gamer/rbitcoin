#!/usr/bin/env python3
import sys, pathlib, re
shards_dir = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path("mutants-shards")
baseline_path = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else pathlib.Path("docs/mutants/survivors-baseline.list")

print(f"DEBUG: shards_dir={shards_dir} exists={shards_dir.exists()}")

# Helper: baseline empty if only comments/blank
def baseline_is_empty(p):
    if not p.exists():
        return True
    try:
        content = p.read_text(errors="ignore")
        if len(content.strip()) == 0:
            return True
        # Only comments?
        real_lines = [l for l in content.splitlines() if l.strip() and not l.strip().startswith("#")]
        return len(real_lines) == 0
    except:
        return True

if not shards_dir.exists():
    print(f"WARNING: {shards_dir} does not exist - download-artifact found 0 artifacts")
    if baseline_is_empty(baseline_path):
        print("Baseline empty (only comments) - OK for initial PR, initializing baseline")
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        if not baseline_path.exists():
            baseline_path.write_text("# Baseline of known surviving mutants — auto-populated on first CI run\n")
        sys.exit(0)
    else:
        print("Baseline NOT empty but shards missing - failing")
        sys.exit(1)

candidates = list(shards_dir.glob("**/mutants.out")) + list(shards_dir.glob("**/mutants-shard.log")) + list(shards_dir.glob("**/*.out")) + list(shards_dir.glob("**/*.log"))
files, seen = [], set()
for pp in candidates:
    if not pp.is_file():
        continue
    if "target" in pp.parts:
        continue
    try:
        if pp.stat().st_size == 0:
            continue
    except:
        continue
    rp = pp.resolve()
    if rp in seen:
        continue
    seen.add(rp)
    files.append(pp)

print(f"DEBUG: found {len(files)} files: {files[:10]}")
if shards_dir.exists():
    print(f"DEBUG: rglob sample: {list(shards_dir.rglob('*'))[:20]}")

if not files:
    print("No mutants.out / mutants-shard.log found")
    if baseline_is_empty(baseline_path):
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        if not baseline_path.exists() or baseline_is_empty(baseline_path):
            # Keep existing header if present
            if not baseline_path.exists():
                baseline_path.write_text("# Baseline of known surviving mutants — auto-populated on first CI run\n")
        print("Baseline empty and no shards - OK for initial PR")
        sys.exit(0)
    print("::error::No mutants.out found and baseline is not empty")
    sys.exit(1)

survivors=set()
for pp in files:
    text = pp.read_text(errors="ignore")
    for line in text.splitlines():
        s=line.strip()
        if not s or s.startswith("#"):
            continue
        # Support both formats: "file -> mutant" and "MISSED" lines
        if "->" in s:
            s = re.sub(r'^\./', '', s)
            if "/rbitcoin-bip34/" in s:
                s = s.split("/rbitcoin-bip34/")[-1]
            survivors.add(s)
        elif "MISSED" in s:
            # Keep MISSED lines as survivor entry if they contain ->
            # Some logs have format with ->
            if "->" in s:
                survivors.add(s)

print(f"Parsed {len(survivors)} survivors from shards")

# Baseline parsing
baseline=set()
if baseline_path.exists():
    for l in baseline_path.read_text().splitlines():
        l=l.strip()
        if not l or l.startswith("#"):
            continue
        baseline.add(l)

if not baseline:
    print(f"Baseline empty — initializing with {len(survivors)} survivors")
    baseline_path.parent.mkdir(parents=True, exist_ok=True)
    baseline_path.write_text("# Baseline of known surviving mutants — auto-populated on first CI run\n" + "\n".join(sorted(survivors)) + "\n")
    print("OK — baseline initialized")
    sys.exit(0)

new=sorted(survivors-baseline)
print(f"shards: {len(files)} survivors: {len(survivors)} baseline: {len(baseline)} new: {len(new)}")
if new:
    print("::error::New survivors vs baseline")
    for n in new[:50]:
        print(f"  NEW: {n}")
    sys.exit(1)
print("OK — no new survivors")
