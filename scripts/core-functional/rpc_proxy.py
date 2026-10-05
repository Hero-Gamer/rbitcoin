#!/usr/bin/env python3
"""Test-only JSON-RPC proxy in front of rbitcoin-node.

Not the operator product. Core functional tests speak to this process.
Node methods are forwarded; `maxfeerate` BTC/kvB is rewritten to sat/vB.
Wallet/utility methods are handled locally in later steps.
"""

from __future__ import annotations

import base64
import hmac
import json
import threading
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Callable

from rpcauth import password_to_hmac


# GBT longpoll can sit ~80s; stay under Core's client-side patience.
FORWARD_TIMEOUT_S = 180.0

# Node maxfeerate is sat/vB. Core tests speak BTC/kvB.
_MAXFEERATE_METHODS = {
    "sendrawtransaction": 1,
    "testmempoolaccept": 1,
    "submitpackage": 1,
}
_CORE_MAXFEERATE_MSG = (
    "Fee rates larger than or equal to 1BTC/kvB are not accepted"
)


def node_authorization(cookie_line: str) -> str:
    """Node TCP is Bearer. TestNode cookie is `__cookie__:<token>`."""
    prefix = "__cookie__:"
    token = cookie_line[len(prefix) :] if cookie_line.startswith(prefix) else cookie_line
    return f"Bearer {token}"


def token_from_cookie_line(cookie: str) -> str:
    prefix = "__cookie__:"
    if cookie.startswith(prefix):
        return cookie[len(prefix) :]
    return cookie


def parse_basic_userpass(authorization: str) -> tuple[str, str] | None:
    if not authorization.startswith("Basic "):
        return None
    try:
        raw = base64.b64decode(authorization[6:]).decode()
    except (ValueError, UnicodeDecodeError):
        return None
    if ":" not in raw:
        return None
    user, password = raw.split(":", 1)
    return user, password


def authorization_ok(authorization: str, cookie_line: str | None) -> bool:
    """Cookie Basic, or any username whose password equals the token.

    Warnet sends `rpcuser:rpcpassword`. The username is not Core `rpcuser`.
    """
    if not cookie_line:
        return True
    want = "Basic " + base64.b64encode(cookie_line.encode()).decode()
    if authorization == want:
        return True
    parsed = parse_basic_userpass(authorization)
    if parsed is None:
        return False
    _user, password = parsed
    return password == token_from_cookie_line(cookie_line)


def parse_rpcauth_line(line: str) -> tuple[str, str, str] | None:
    """Core `rpcauth=user:salt$hash` (HMAC-SHA256, salt is the key)."""
    raw = line.strip()
    if ":" not in raw or "$" not in raw:
        return None
    user, rest = raw.split(":", 1)
    salt, mac = rest.split("$", 1)
    if not user or not salt or not mac:
        return None
    return user, salt, mac


def parse_whitelist_line(line: str) -> tuple[str, set[str]] | None:
    """Core `rpcwhitelist=user:method,method`."""
    raw = line.strip()
    if ":" not in raw:
        return None
    user, methods = raw.split(":", 1)
    user = user.strip()
    if not user:
        return None
    return user, {m.strip() for m in methods.split(",") if m.strip()}


def whitelist_map(lines: list[str]) -> dict[str, set[str]]:
    out: dict[str, set[str]] = {}
    for line in lines:
        parsed = parse_whitelist_line(line)
        if parsed is None:
            continue
        user, methods = parsed
        # Core set-intersects a later rpcwhitelist line for the same user.
        if user in out:
            out[user] &= methods
        else:
            out[user] = methods
    return out


def _hmac_ok(salt: str, password: str, expected: str) -> bool:
    got = password_to_hmac(salt, password)
    if len(got) != len(expected):
        return False
    return hmac.compare_digest(got, expected)


def _others_restricted(whitelist_default: str | None, has_entries: bool) -> bool:
    """Core: default 0 limits only users who have a whitelist entry.

    Unset, or any other value, subjects every other user to an empty list
    once a whitelist exists. Explicit non-zero with no entries denies all.
    """
    if whitelist_default is None:
        return has_entries
    if whitelist_default.strip().lower() in ("0", "false", "no", "off"):
        return False
    return True


def method_permitted(
    user: str,
    method: str | None,
    entries: dict[str, set[str]],
    whitelist_default: str | None,
) -> bool:
    if user in entries:
        return method is not None and method in entries[user]
    if _others_restricted(whitelist_default, bool(entries)):
        return False
    return True


def authenticated_user(
    authorization: str,
    cookie_line: str | None,
    rpcauth_lines: list[str],
) -> str | None:
    """Username that passed cookie, tank-token, or `rpcauth` HMAC.

    No cookie and no `rpcauth` lines stays open (the functional harness
    before `.cookie` exists). A cookie line refuses a password that matches
    neither the token nor an `rpcauth` row.
    """
    parsed = parse_basic_userpass(authorization)
    if not cookie_line and not rpcauth_lines:
        return parsed[0] if parsed else ""
    if cookie_line:
        want = "Basic " + base64.b64encode(cookie_line.encode()).decode()
        if authorization == want:
            return "__cookie__"
        if parsed is not None and parsed[1] == token_from_cookie_line(cookie_line):
            return parsed[0]
    if parsed is None:
        return "" if not cookie_line else None
    user, password = parsed
    # Core keeps scanning rpcauth rows for this user (password rotation).
    saw_user = False
    for line in rpcauth_lines:
        rec = parse_rpcauth_line(line)
        if rec is None or rec[0] != user:
            continue
        saw_user = True
        if _hmac_ok(rec[1], password, rec[2]):
            return user
    if saw_user or cookie_line:
        return None
    return user


def rpc_methods(payload: Any) -> list[str | None]:
    if isinstance(payload, dict):
        method = payload.get("method")
        return [method if isinstance(method, str) else None]
    if isinstance(payload, list):
        out: list[str | None] = []
        for item in payload:
            if isinstance(item, dict) and isinstance(item.get("method"), str):
                out.append(item["method"])
            else:
                out.append(None)
        return out or [None]
    return [None]


def request_authorized(
    authorization: str,
    cookie_line: str | None,
    rpcauth_lines: list[str],
    whitelist_lines: list[str],
    whitelist_default: str | None,
    payload: Any,
) -> str:
    """`ok`, `unauthorized`, or `forbidden` for one HTTP RPC body."""
    user = authenticated_user(authorization, cookie_line, rpcauth_lines)
    if user is None:
        return "unauthorized"
    entries = whitelist_map(whitelist_lines)
    for method in rpc_methods(payload):
        if not method_permitted(user, method, entries, whitelist_default):
            return "forbidden"
    return "ok"


def core_btc_kvb_to_sat_vb(value: Any) -> int:
    """Core `maxfeerate` BTC/kvB → node sat/vB. `>= 1` is Core `-8`."""
    if value is None:
        raise RpcError(-8, "Invalid amount")
    if isinstance(value, bool):
        raise RpcError(-8, "Invalid amount")
    if isinstance(value, int):
        btc = float(value)
    elif isinstance(value, float):
        btc = value
    elif isinstance(value, str):
        try:
            btc = float(value.strip())
        except ValueError as e:
            raise RpcError(-8, "Invalid amount") from e
    else:
        raise RpcError(-8, "Invalid amount")
    if btc < 0:
        raise RpcError(-8, "Amount out of range")
    if btc >= 1:
        raise RpcError(-8, _CORE_MAXFEERATE_MSG)
    return int(btc * 100_000)


def named_param_index(method: str, key: str) -> int | None:
    """Positional index for a Core named key on a forwarded method."""
    if key == "maxfeerate":
        return _MAXFEERATE_METHODS.get(method)
    if key == "maxburnamount" and method in _MAXFEERATE_METHODS:
        return 2
    return None


def peel_authproxy_args(item: dict[str, Any]) -> None:
    """AuthServiceProxy mixed call → positional list the node will accept.

    `submitpackage([...], maxfeerate=0)` arrives as `{args: [[...]], maxfeerate: 0}`.
    The node rejects named `args` except on `echo` (which keeps this object).
    """
    params = item.get("params")
    if not isinstance(params, dict) or "args" not in params:
        return
    args = params.get("args")
    if not isinstance(args, list):
        return
    method = item.get("method") if isinstance(item.get("method"), str) else ""
    named = {k: v for k, v in params.items() if k != "args"}
    if not named:
        item["params"] = list(args)
        return
    pos = list(args)
    for k, v in named.items():
        idx = named_param_index(method, k)
        if idx is None:
            return
        while len(pos) <= idx:
            pos.append(None)
        pos[idx] = v
    item["params"] = pos


_TMA_ABORT = frozenset(
    {
        "missing-inputs",
        "max-fee-exceeded",
        "bip125-replacement-disallowed",
    }
)


def shim_gettxoutsetinfo(height: Any, bestblock: Any) -> dict[str, Any]:
    """Harness stand-in. No UTXO set, so `txouts` is -1."""
    return {"height": height, "bestblock": bestblock, "txouts": -1}


def rewrite_testmempoolaccept_abort(method: Any, parsed: dict[str, Any]) -> None:
    """Core PCKG abort: first abort-class row keeps reject-reason; others id-only."""
    if method != "testmempoolaccept":
        return
    result = parsed.get("result")
    if not isinstance(result, list) or len(result) < 2:
        return
    idx = None
    for i, row in enumerate(result):
        if isinstance(row, dict) and row.get("reject-reason") in _TMA_ABORT:
            idx = i
            break
    if idx is None:
        return
    for i, row in enumerate(result):
        if i == idx or not isinstance(row, dict):
            continue
        result[i] = {"txid": row.get("txid"), "wtxid": row.get("wtxid")}


def rewrite_getmempoolinfo_budget(method: Any, parsed: dict[str, Any]) -> None:
    """Core `maxmempool` is the byte cap. The node field is the weight budget.

    The bitcoind shim maps `-maxmempool=N` to a 4× weight budget so vsize
    capacity matches Core. `bytes` stays virtual size, so the reported cap
    is weight/4 (`mempool_limit.py` compares `maxmempool - bytes`).
    """
    if method != "getmempoolinfo":
        return
    result = parsed.get("result")
    if not isinstance(result, dict):
        return
    cap = result.get("maxmempool")
    if isinstance(cap, int) and not isinstance(cap, bool):
        result["maxmempool"] = cap // 4


def _rewrite_forwarded_body(method: Any, body: bytes) -> bytes:
    if method == "getmempoolinfo":
        try:
            parsed = json.loads(body.decode())
        except (UnicodeDecodeError, json.JSONDecodeError):
            return body
        if not isinstance(parsed, dict) or not isinstance(parsed.get("result"), dict):
            return body
        rewrite_getmempoolinfo_budget(method, parsed)
        return json.dumps(parsed).encode()
    return _rewrite_forwarded_testmempoolaccept(method, body)


def _rewrite_forwarded_testmempoolaccept(method: Any, body: bytes) -> bytes:
    if method != "testmempoolaccept":
        return body
    try:
        parsed = json.loads(body.decode())
    except (UnicodeDecodeError, json.JSONDecodeError):
        return body
    if not isinstance(parsed, dict):
        return body
    result = parsed.get("result")
    if not isinstance(result, list) or len(result) < 2:
        return body
    if not any(
        isinstance(row, dict) and row.get("reject-reason") in _TMA_ABORT
        for row in result
    ):
        return body
    rewrite_testmempoolaccept_abort(method, parsed)
    return json.dumps(parsed).encode()


def rewrite_core_maxfeerate(item: dict[str, Any]) -> None:
    method = item.get("method")
    idx = _MAXFEERATE_METHODS.get(method) if isinstance(method, str) else None
    if idx is None:
        return
    params = item.get("params", [])
    if isinstance(params, list):
        if len(params) > idx and params[idx] is not None:
            params[idx] = core_btc_kvb_to_sat_vb(params[idx])
    elif isinstance(params, dict) and "maxfeerate" in params:
        params["maxfeerate"] = core_btc_kvb_to_sat_vb(params["maxfeerate"])


class RpcError(Exception):
    def __init__(self, code: int, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


class RpcProxy:
    """HTTP JSON-RPC server that forwards to an internal rbitcoin-node."""

    def __init__(
        self,
        listen: tuple[str, int],
        node_url: str,
        cookie_line: Callable[[], str | None],
        rpcauth_lines: list[str] | None = None,
        whitelist_lines: list[str] | None = None,
        whitelist_default: str | None = None,
    ) -> None:
        self.node_url = node_url.rstrip("/") + "/"
        self.cookie_line = cookie_line
        self.rpcauth_lines = list(rpcauth_lines or [])
        self.whitelist_lines = list(whitelist_lines or [])
        self.whitelist_default = whitelist_default
        self._handlers: dict[str, Callable[[Any], dict[str, Any]]] = {}
        proxy = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _fmt: str, *_args: object) -> None:
                return

            def do_POST(self) -> None:
                length = int(self.headers.get("Content-Length", "0"))
                raw = self.rfile.read(length) if length else b""
                auth = self.headers.get("Authorization", "")
                status, body = proxy.handle_http(raw, auth)
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self._httpd = ThreadingHTTPServer(listen, Handler)
        self._thread: threading.Thread | None = None

    def register(self, method: str, fn: Callable[[Any], dict[str, Any]]) -> None:
        self._handlers[method] = fn

    def start(self) -> None:
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)
        self._thread.start()

    def shutdown(self) -> None:
        self._httpd.shutdown()
        if self._thread is not None:
            self._thread.join(timeout=2)

    def handle_http(self, raw: bytes, authorization: str) -> tuple[int, bytes]:
        cookie = self.cookie_line()
        try:
            payload = json.loads(raw.decode() or "null")
            parsed = True
        except (UnicodeDecodeError, json.JSONDecodeError):
            payload = None
            parsed = False
        decision = request_authorized(
            authorization,
            cookie,
            self.rpcauth_lines,
            self.whitelist_lines,
            self.whitelist_default,
            payload,
        )
        if decision == "unauthorized":
            return 401, b'{"error":"unauthorized"}\n'
        if decision == "forbidden":
            return 403, b'{"error":"forbidden"}\n'
        if not parsed:
            return self.forward_raw(raw)
        try:
            if isinstance(payload, list):
                for item in payload:
                    if isinstance(item, dict):
                        peel_authproxy_args(item)
                        rewrite_core_maxfeerate(item)
                return self.forward_raw(json.dumps(payload).encode())
            if isinstance(payload, dict):
                peel_authproxy_args(payload)
                rewrite_core_maxfeerate(payload)
                method = payload.get("method")
                if isinstance(method, str) and method in self._handlers:
                    return 200, json.dumps(self._one(payload)).encode()
                status, body = self.forward_raw(json.dumps(payload).encode())
                return status, _rewrite_forwarded_body(method, body)
        except RpcError as e:
            req_id = payload.get("id") if isinstance(payload, dict) else None
            body = json.dumps(
                {
                    "result": None,
                    "error": {"code": e.code, "message": e.message},
                    "id": req_id,
                }
            ).encode()
            return 200, body
        return self.forward_raw(raw)

    def forward_raw(self, raw: bytes) -> tuple[int, bytes]:
        cookie = self.cookie_line() or ""
        req = urllib.request.Request(
            self.node_url,
            data=raw,
            headers={
                "Authorization": node_authorization(cookie),
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=FORWARD_TIMEOUT_S) as resp:
                return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read() if e.fp else b""
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            body = json.dumps(
                {
                    "result": None,
                    "error": {"code": -28, "message": f"Loading... ({e})"},
                    "id": None,
                }
            ).encode()
            return 200, body

    def _one(self, item: Any) -> dict[str, Any]:
        if not isinstance(item, dict):
            return {
                "result": None,
                "error": {"code": -32600, "message": "Invalid request"},
                "id": None,
            }
        req_id = item.get("id")
        method = item.get("method")
        params = item.get("params", [])
        if not isinstance(method, str):
            return {
                "result": None,
                "error": {"code": -32600, "message": "Invalid request"},
                "id": req_id,
            }
        local = self._handlers.get(method)
        if local is not None:
            try:
                result = local(params)
            except RpcError as e:
                return {
                    "result": None,
                    "error": {"code": e.code, "message": e.message},
                    "id": req_id,
                }
            except Exception as e:  # noqa: BLE001 — surface as RPC error
                return {
                    "result": None,
                    "error": {"code": -1, "message": str(e)},
                    "id": req_id,
                }
            if isinstance(result, dict) and "error" in result and "result" in result:
                result.setdefault("id", req_id)
                return result
            return {"result": result, "error": None, "id": req_id}
        return self.forward(item)

    def forward(self, item: dict[str, Any]) -> dict[str, Any]:
        cookie = self.cookie_line() or ""
        body = json.dumps(item).encode()
        req = urllib.request.Request(
            self.node_url,
            data=body,
            headers={
                "Authorization": node_authorization(cookie),
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=FORWARD_TIMEOUT_S) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                return json.loads(raw.decode())
            except (UnicodeDecodeError, json.JSONDecodeError):
                return {
                    "result": None,
                    "error": {"code": -1, "message": f"HTTP {e.code}"},
                    "id": item.get("id"),
                }
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            # Core wait_for_rpc_connection retries -28 / -342 only. A
            # forwarded "node not listening yet" must look like warmup, not
            # a fatal -1 (restart_node races the proxy vs rbitcoin-node).
            return {
                "result": None,
                "error": {
                    "code": -28,
                    "message": f"Loading... ({e})",
                },
                "id": item.get("id"),
            }
        try:
            parsed = json.loads(raw.decode())
        except (UnicodeDecodeError, json.JSONDecodeError):
            return {
                "result": None,
                "error": {"code": -1, "message": "node returned non-JSON"},
                "id": item.get("id"),
            }
        if isinstance(parsed, dict):
            rewrite_testmempoolaccept_abort(item.get("method"), parsed)
            return parsed
        return {
            "result": None,
            "error": {"code": -1, "message": "node returned non-object"},
            "id": item.get("id"),
        }


def _offset_port(public_rpc: int, offset: int) -> int:
    """Shift a Core-assigned RPC port; wrap instead of overflowing 65535."""
    p = public_rpc + offset
    if p <= 65535:
        return p
    p = public_rpc - offset
    if p >= 1:
        return p
    return max(1, public_rpc - 1)


def node_rpc_port(public_rpc: int) -> int:
    """Internal node RPC. Public port stays on the proxy."""
    return _offset_port(public_rpc, 10_000)


def esplora_port(public_rpc: int) -> int:
    """Esplora listen for the test wallet shim (Step 18).

    Must not sit next to ``node_rpc_port``: Core assigns consecutive
    ``-rpcport`` values, so ``node_rpc(n) + 1 == node_rpc(n + 1)``. That
    collision made the next node's proxy POST ``getblockcount`` at the
    previous node's Esplora (HTTP 404).
    """
    return _offset_port(public_rpc, 20_000)
