#!/usr/bin/env bash
# Two-node cjdns TUN that UDP-peers on 127.0.0.1. Fail if TUN cannot be created.
set -euo pipefail

ROOT="${1:?dir}"
mkdir -p "$ROOT"

if [[ ! -e /dev/net/tun ]]; then
  sudo modprobe tun || true
fi
if [[ ! -e /dev/net/tun ]]; then
  echo "cjdns: /dev/net/tun missing — no silent 127.0.0.1 fallback" >&2
  exit 1
fi
if ! sudo -n true 2>/dev/null; then
  echo "cjdns: passwordless sudo required for TUN" >&2
  exit 1
fi

free_port() {
  python3 -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

UDP_A="$(free_port)"
UDP_B="$(free_port)"
ADMIN_A="$(free_port)"
ADMIN_B="$(free_port)"

cjdroute --genconf | cjdroute --cleanconf >"$ROOT/a.raw.json"
cjdroute --genconf | cjdroute --cleanconf >"$ROOT/b.raw.json"

python3 - "$ROOT" "$UDP_A" "$UDP_B" "$ADMIN_A" "$ADMIN_B" <<'PY'
import json, sys
root, udp_a, udp_b, admin_a, admin_b = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])

def load(name):
    with open(f"{root}/{name}.raw.json") as f:
        return json.load(f)

a, b = load("a"), load("b")
pw = a["authorizedPasswords"][0]["password"]
a_pub = a["publicKey"]
b_pub = b["publicKey"]

def udp(cfg):
    return cfg["interfaces"]["UDPInterface"][0]

udp(a)["bind"] = f"0.0.0.0:{udp_a}"
udp(b)["bind"] = f"0.0.0.0:{udp_b}"
udp(a)["connectTo"] = {
    f"127.0.0.1:{udp_b}": {
        "password": b["authorizedPasswords"][0]["password"],
        "publicKey": b_pub,
        "peerName": "b",
    }
}
udp(b)["connectTo"] = {
    f"127.0.0.1:{udp_a}": {"password": pw, "publicKey": a_pub, "peerName": "a"}
}
# One UDP bind (IPv4 loopback). Drop the IPv6 UDPInterface row.
a["interfaces"]["UDPInterface"] = [udp(a)]
b["interfaces"]["UDPInterface"] = [udp(b)]
a["router"]["dnsSeeds"] = []
b["router"]["dnsSeeds"] = []
a["security"] = [{"keepNetAdmin": 1}, {"noforks": 1}, {"setupComplete": 1}]
b["security"] = [{"keepNetAdmin": 1}, {"noforks": 1}, {"setupComplete": 1}]
a["logging"] = {"logTo": "stdout"}
b["logging"] = {"logTo": "stdout"}
a["admin"]["bind"] = f"127.0.0.1:{admin_a}"
b["admin"]["bind"] = f"127.0.0.1:{admin_b}"
a["pipe"] = f"{root}/a.cjdns.sock"
b["pipe"] = f"{root}/b.cjdns.sock"

def tun(cfg, dev):
    iface = cfg.setdefault("router", {}).setdefault("interface", {})
    iface["type"] = "TUNInterface"
    iface["tunDevice"] = dev
    cfg["noBackground"] = 1

tun(a, "rbtc0")
tun(b, "rbtc1")

for name, cfg in (("a", a), ("b", b)):
    with open(f"{root}/{name}.conf", "w") as f:
        json.dump(cfg, f, indent=2)
        f.write("\n")
    with open(f"{root}/{name}.ipv6", "w") as f:
        f.write(cfg["ipv6"] + "\n")
PY

CJDNS_BIN="$(command -v cjdroute)"

# cjdroute reads stdin until EOF as JSON conf, then `--nobg` keeps the
# client in the event loop so the core is not reaped (parent would
# otherwise `return 0`). setsid: cargo/nix must not SIGHUP the pair.
start_cjdroute() {
  local name="$1"
  sudo bash -c 'exec setsid "$1" --nobg <"$2" >"$3" 2>&1' \
    _ "$CJDNS_BIN" "$ROOT/$name.conf" "$ROOT/$name.log" &
  echo $! >"$ROOT/$name.pid"
}

wait_tun() {
  local dev="$1"
  local i
  for i in $(seq 1 50); do
    if ip link show "$dev" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  echo "cjdns: $dev never appeared" >&2
  return 1
}

IPV6_A="$(cat "$ROOT/a.ipv6")"
IPV6_B="$(cat "$ROOT/b.ipv6")"

cjdns_alive() {
  [[ -f "$ROOT/$1.pid" ]] && kill -0 "$(cat "$ROOT/$1.pid")" 2>/dev/null
}

# Kernel TCP over the TUNs (the product path). ICMP through cjdns is not
# reliable on GitHub-hosted runners even when the ifaces are up.
cjdns_tcp() {
  python3 - "$IPV6_A" "$IPV6_B" <<'PY'
import socket, sys, time
a, b = sys.argv[1], sys.argv[2]
end = time.time() + 60
last = ""
while time.time() < end:
    srv = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        srv.bind((b, 0))
        srv.listen(1)
        srv.settimeout(1)
        port = srv.getsockname()[1]
        cli = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        try:
            cli.bind((a, 0))
            cli.settimeout(1)
            cli.connect((b, port))
            conn, _ = srv.accept()
            conn.close()
            sys.exit(0)
        except OSError as e:
            last = str(e)
        finally:
            cli.close()
    except OSError as e:
        last = str(e)
    finally:
        srv.close()
    time.sleep(0.4)
print(f"cjdns TCP timeout ({a} -> {b}): {last}", file=sys.stderr)
sys.exit(1)
PY
}

restart_cjdns() {
  local name
  for name in a b; do
    if [[ -f "$ROOT/$name.pid" ]]; then
      sudo kill "$(cat "$ROOT/$name.pid")" 2>/dev/null || true
    fi
  done
  sudo ip link delete rbtc0 2>/dev/null || true
  sudo ip link delete rbtc1 2>/dev/null || true
  sleep 0.5
  start_cjdroute a
  wait_tun rbtc0
  start_cjdroute b
  wait_tun rbtc1
}

# cjdroute can segfault ~1s after the first TCP on GitHub runners.
# Restart the pair instead of failing the mesh on that crash.
mesh_ok=0
for attempt in 1 2 3; do
  if [[ "$attempt" -eq 1 ]]; then
    start_cjdroute a
    wait_tun rbtc0
    start_cjdroute b
    wait_tun rbtc1
  else
    echo "cjdns: restart attempt ${attempt}" >&2
    restart_cjdns
  fi
  if ! cjdns_tcp; then
    echo "cjdns TCP timeout ($IPV6_A <-> $IPV6_B)" >&2
    if [[ "$attempt" -eq 3 ]]; then
      for name in a b; do
        echo "--- $name.log ---" >&2
        tail -n 40 "$ROOT/$name.log" >&2 || true
      done
      exit 1
    fi
    continue
  fi
  dead=""
  for name in a b; do
    if ! cjdns_alive "$name"; then
      echo "cjdns: $name not running after TCP wait" >&2
      tail -n 40 "$ROOT/$name.log" >&2 || true
      dead="$name"
    fi
  done
  if [[ -z "$dead" ]]; then
    mesh_ok=1
    break
  fi
done
if [[ "$mesh_ok" -ne 1 ]]; then
  exit 1
fi
# GHA: angel/core can crash ~1s after the first TCP; require the addrs
# still bind after a settle so we do not hand a dead TUN to the journey.
sleep 2
python3 - "$IPV6_A" "$IPV6_B" <<'PY'
import socket, sys
for ip in sys.argv[1:]:
    s = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
    try:
        s.bind((ip, 0))
    except OSError as e:
        print(f"cjdns: bind {ip} after settle: {e}", file=sys.stderr)
        sys.exit(1)
    finally:
        s.close()
PY

cat >"$ROOT/env" <<EOF
OVERLAY_CJDNS_A=${IPV6_A}
OVERLAY_CJDNS_B=${IPV6_B}
EOF
cat "$ROOT/env"
