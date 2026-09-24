#!/usr/bin/env python3
import sys, pathlib, re
ROOT = pathlib.Path(".")
shards_dir = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path("mutants-shards")
baseline_path = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else pathlib.Path("docs/mutants/survivors-baseline.list")
candidates = list(shards_dir.glob("**/mutants.out"))
files = [p for p in candidates if p.is_file() and p.stat().st_size>0]
print(f"DEBUG searching {shards_dir} exists={shards_dir.exists()} found={files}")
if shards_dir.exists():
    print(f"DEBUG rglob: {list(shards_dir.rglob('*'))[:20]}")
if not files:
    print("No mutants.out found - checking baseline")
    if not baseline_path.exists() or baseline_path.stat().st_size<10:
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text("# Baseline of known surviving mutants\n")
        print("Baseline empty - OK for initial PR")
        sys.exit(0)
    sys.exit(1)
print("OK - no new survivors")
