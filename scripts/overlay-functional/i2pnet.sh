#!/usr/bin/env bash
# Private i2pd mesh (netid ≠ 2). Empty public reseed. SAM on two routers.
set -euo pipefail

ROOT="${1:?dir}"
NETID="${OVERLAY_I2P_NETID:-16}"
N="${OVERLAY_I2P_NODES:-6}"
mkdir -p "$ROOT"

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

write_conf() {
  local dir="$1" ntcp="$2" sam="$3" ipv4="$4"
  mkdir -p "$dir/netDb"
  cat >"$dir/i2pd.conf" <<EOF
loglevel = error
log = file
logfile = $dir/i2pd.log
pidfile = $dir/i2pd.pid
ipv4 = true
ipv6 = false
notransit = false
floodfill = true
bandwidth = P
netid = $NETID
ssu = false
ifname4 = lo
address4 = $ipv4
host = $ipv4
reservedrange = false

[ntcp2]
enabled = true
published = true
port = $ntcp

[ssu2]
enabled = false

[sam]
enabled = true
address = 127.0.0.1
port = $sam

[http]
enabled = false

[httpproxy]
enabled = false

[socksproxy]
enabled = false

[i2pcontrol]
enabled = false

[reseed]
urls =
yggurls =
threshold = 1

[exploratory]
inbound.length = 1
inbound.quantity = 1
outbound.length = 1
outbound.quantity = 1
EOF
}

NTCPS=()
SAMS=()
: >"$ROOT/loopbacks"
for i in $(seq 0 $((N - 1))); do
  dir="$ROOT/n$i"
  ntcp="$(free_port)"
  sam="$(free_port)"
  ipv4="127.0.0.$((i + 2))"
  NTCPS+=("$ntcp")
  SAMS+=("$sam")
  echo "$ipv4" >>"$ROOT/loopbacks"
  # /32, not /8. A second 127.0.0.0/8 overlaps the primary loopback and
  # local delivery of SAM on 127.0.0.1 becomes unstable.
  if ! ip -4 addr show dev lo | grep -q "inet ${ipv4}/"; then
    sudo ip addr add "$ipv4/32" dev lo
  fi
  write_conf "$dir" "$ntcp" "$sam" "$ipv4"
  i2pd --datadir "$dir" --conf "$dir/i2pd.conf" --address4 "$ipv4" --daemon
done

deadline=$((SECONDS + 30))
for i in $(seq 0 $((N - 1))); do
  while [[ ! -f "$ROOT/n$i/router.info" ]]; do
    if (( SECONDS >= deadline )); then
      echo "i2pd n$i router.info missing" >&2
      exit 1
    fi
    sleep 0.2
  done
done

python3 - "$ROOT" "$N" <<'PY'
import sys, zipfile
from pathlib import Path
root = Path(sys.argv[1])
n = int(sys.argv[2])
zpath = root / "reseed.zip"
with zipfile.ZipFile(zpath, "w", compression=zipfile.ZIP_STORED) as z:
    for i in range(n):
        ri = root / f"n{i}" / "router.info"
        z.write(ri, arcname=f"routerInfo-n{i}.dat")
print(zpath)
PY

wait_dead() {
  local pid="$1" i
  for i in $(seq 1 50); do
    if ! kill -0 "$pid" 2>/dev/null; then
      return 0
    fi
    sleep 0.2
  done
  kill -9 "$pid" 2>/dev/null || true
  sleep 0.2
}

port_open() {
  python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(0.2); r=s.connect_ex(("127.0.0.1", int(sys.argv[1]))); sys.exit(0 if r==0 else 1)' "$1"
}

for i in $(seq 0 $((N - 1))); do
  if [[ -f "$ROOT/n$i/i2pd.pid" ]]; then
    pid="$(cat "$ROOT/n$i/i2pd.pid")"
    kill "$pid" 2>/dev/null || true
    wait_dead "$pid"
  fi
done
for port in "${SAMS[@]}"; do
  for _ in $(seq 1 50); do
    if ! port_open "$port"; then
      break
    fi
    sleep 0.2
  done
done
for i in $(seq 0 $((N - 1))); do
  ipv4="127.0.0.$((i + 2))"
  i2pd --datadir "$ROOT/n$i" --conf "$ROOT/n$i/i2pd.conf" \
    --address4 "$ipv4" --reseed.zipfile "$ROOT/reseed.zip" --daemon
done

# HELLO returns before SESSION CREATE is stable. Do not probe SESSION
# CREATE here: a short recv closes the socket and i2pd logs EOF instead
# of finishing the session.
sam_hello() {
  local port="$1" wall="$2"
  python3 - "$port" "$wall" <<'PY'
import socket, sys, time
port = int(sys.argv[1])
wall = int(sys.argv[2])
end = time.time() + wall
last = ""
while time.time() < end:
    try:
        s = socket.create_connection(("127.0.0.1", port), 2)
        s.sendall(b"HELLO VERSION MIN=3.1 MAX=3.1\n")
        last = s.recv(1024).decode("utf-8", "replace")
        s.close()
        if "RESULT=OK" in last.upper():
            sys.exit(0)
    except OSError as e:
        last = str(e)
    time.sleep(0.4)
print(f"SAM {port} not ready: {last}", file=sys.stderr)
sys.exit(1)
PY
}

sam_hello "${SAMS[0]}" 180
sam_hello "${SAMS[1]}" 180
sleep 2

cat >"$ROOT/env" <<EOF
OVERLAY_I2P_SAM=127.0.0.1:${SAMS[0]}
OVERLAY_I2P_SAM_B=127.0.0.1:${SAMS[1]}
EOF
cat "$ROOT/env"
