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

# Open the conf inside sudo. `sudo cjdroute <conf &` often gets a closed
# stdin on GitHub Actions, so the Angel cancels the core immediately.
start_cjdroute() {
  local name="$1"
  sudo bash -c 'exec "$1" <"$2" >"$3" 2>&1' _ "$CJDNS_BIN" "$ROOT/$name.conf" "$ROOT/$name.log" &
  echo $! >"$ROOT/$name.pid"
}

start_cjdroute a
sleep 1
start_cjdroute b

IPV6_A="$(cat "$ROOT/a.ipv6")"
IPV6_B="$(cat "$ROOT/b.ipv6")"

# Kernel TCP over the TUNs (the product path). ICMP through cjdns is not
# reliable on GitHub-hosted runners even when the ifaces are up.
if ! python3 - "$IPV6_A" "$IPV6_B" <<'PY'
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
then
  echo "cjdns TCP timeout ($IPV6_A <-> $IPV6_B)" >&2
  for name in a b; do
    echo "--- $name pid ---" >&2
    if [[ -f "$ROOT/$name.pid" ]] && kill -0 "$(cat "$ROOT/$name.pid")" 2>/dev/null; then
      echo "alive $(cat "$ROOT/$name.pid")" >&2
    else
      echo "dead" >&2
    fi
    echo "--- $name.log ---" >&2
    tail -n 80 "$ROOT/$name.log" >&2 || true
  done
  sudo ip -6 addr show rbtc0 >&2 || true
  sudo ip -6 addr show rbtc1 >&2 || true
  sudo ip -6 route get "$IPV6_B" >&2 || true
  exit 1
fi

cat >"$ROOT/env" <<EOF
OVERLAY_CJDNS_A=${IPV6_A}
OVERLAY_CJDNS_B=${IPV6_B}
EOF
cat "$ROOT/env"
