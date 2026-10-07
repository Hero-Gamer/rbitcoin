#!/usr/bin/env python3
"""Plant at most N shape-0 script_kernel inputs from Core script_tests.json.

Grammar: 0xFE || flags:u32le || shape 0 || version:i32le || sequence:u32le ||
locktime:u32le || amount:i64le || scriptSig || scriptPubKey || witness.
Version, sequence, and locktime match Core's script_tests spend (1, max, 0).
Rows naming OP_TOALTSTACK, OP_TUCK, or CHECKMULTISIG come first; the rest
fill from the start of the file. Assembly follows core_script.rs.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OPCODE_RS = ROOT / "crates/rbitcoin-consensus/src/script/core_script.rs"
MAX_PUSH = 10_000
MAX_WIT = 32
MAX_LEN = 2000
PREFERRED = ("OP_TOALTSTACK", "OP_TUCK", "CHECKMULTISIG")

FLAG_BITS = {
    "P2SH": 1 << 0,
    "STRICTENC": 1 << 1,
    "DERSIG": 1 << 2,
    "LOW_S": 1 << 3,
    "NULLDUMMY": 1 << 4,
    "SIGPUSHONLY": 1 << 5,
    "MINIMALDATA": 1 << 6,
    "DISCOURAGE_UPGRADABLE_NOPS": 1 << 7,
    "CLEANSTACK": 1 << 8,
    "CHECKLOCKTIMEVERIFY": 1 << 9,
    "CHECKSEQUENCEVERIFY": 1 << 10,
    "WITNESS": 1 << 11,
    "DISCOURAGE_UPGRADABLE_WITNESS_PROGRAM": 1 << 12,
    "MINIMALIF": 1 << 13,
    "NULLFAIL": 1 << 14,
    "WITNESS_PUBKEYTYPE": 1 << 15,
    "CONST_SCRIPTCODE": 1 << 16,
    "TAPROOT": 1 << 17,
    "DISCOURAGE_UPGRADABLE_TAPROOT_VERSION": 1 << 18,
    "DISCOURAGE_OP_SUCCESS": 1 << 19,
    "DISCOURAGE_UPGRADABLE_PUBKEYTYPE": 1 << 20,
}


def opcode_map(path: Path) -> dict[str, int]:
    text = path.read_text()
    found = re.findall(r'\("([^"]+)",\s*(0x[0-9a-fA-F]+)\)', text)
    if not found:
        raise SystemExit(f"no opcodes in {path}")
    return {name: int(value, 16) for name, value in found}


def push_data(out: bytearray, data: bytes) -> None:
    n = len(data)
    if n < 0x4C:
        out.append(n)
    elif n <= 0xFF:
        out.append(0x4C)
        out.append(n)
    elif n <= 0xFFFF:
        out.append(0x4D)
        out += n.to_bytes(2, "little")
    else:
        out.append(0x4E)
        out += n.to_bytes(4, "little")
    out += data


def encode_scriptnum(n: int) -> bytes:
    if n == 0:
        return b""
    neg = n < 0
    if neg:
        n = -n
    out = bytearray()
    while n > 0:
        out.append(n & 0xFF)
        n >>= 8
    if out[-1] & 0x80:
        out.append(0x80 if neg else 0x00)
    elif neg:
        out[-1] |= 0x80
    return bytes(out)


def assemble(src: str, ops: dict[str, int]) -> bytes:
    out = bytearray()
    i = 0
    b = src.encode()
    while i < len(b):
        while i < len(b) and chr(b[i]).isspace():
            i += 1
        if i >= len(b):
            break
        if b[i] == ord("'"):
            i += 1
            start = i
            while i < len(b) and b[i] != ord("'"):
                i += 1
            if i >= len(b):
                raise ValueError("unterminated string")
            push_data(out, src.encode()[start:i])
            i += 1
            continue
        if b[i] == ord("0") and i + 1 < len(b) and b[i + 1] in (ord("x"), ord("X")):
            i += 2
            start = i
            while i < len(b) and chr(b[i]).lower() in "0123456789abcdef":
                i += 1
            hexpart = src[start:i]
            if len(hexpart) % 2:
                raise ValueError(f"odd hex: {hexpart}")
            out += bytes.fromhex(hexpart)
            continue
        start = i
        while i < len(b) and not chr(b[i]).isspace():
            i += 1
        tok = src[start:i]
        if not tok:
            continue
        try:
            n = int(tok, 10)
        except ValueError:
            n = None
        if n is not None:
            if n == 0:
                out.append(0x00)
            elif n == -1:
                out.append(0x4F)
            elif 1 <= n <= 16:
                out.append(0x50 + n)
            else:
                push_data(out, encode_scriptnum(n))
            continue
        op = ops.get(tok)
        if op is None:
            for name, value in ops.items():
                if name.lower() == tok.lower():
                    op = value
                    break
        if op is None:
            raise ValueError(f"unknown token {tok}")
        out.append(op)
    return bytes(out)


def parse_flags(text: str) -> int:
    if not text or text.upper() == "NONE":
        return 0
    flags = 0
    for part in text.split(","):
        name = part.strip().upper()
        if not name:
            continue
        if name == "TAPSCRIPT":
            flags |= FLAG_BITS["TAPROOT"] | FLAG_BITS["WITNESS"]
            continue
        bit = FLAG_BITS.get(name)
        if bit is not None:
            flags |= bit
    return flags


def witness_and_amount(cell, ops: dict[str, int]) -> tuple[list[bytes], int]:
    if not cell:
        return [], 0
    amount = 0
    stack: list[bytes] = []
    for i, item in enumerate(cell):
        if isinstance(item, (int, float)) and not isinstance(item, bool):
            if i == len(cell) - 1:
                amount = int(round(float(item) * 100_000_000))
                continue
        if isinstance(item, str):
            if item == "":
                stack.append(b"")
                continue
            if item.startswith("#SCRIPT#"):
                stack.append(assemble(item[len("#SCRIPT#") :].strip(), ops))
                continue
            if item in ("#CONTROLBLOCK#",) or "TAPROOTOUTPUT" in item:
                raise ValueError(item)
            hexpart = item[2:] if item[:2].lower() == "0x" else item
            if len(hexpart) % 2 or any(c not in "0123456789abcdefABCDEF" for c in hexpart):
                raise ValueError(f"witness is not hex: {item}")
            stack.append(bytes.fromhex(hexpart))
            continue
        if isinstance(item, (int, float)) and not isinstance(item, bool):
            amount = int(round(float(item) * 100_000_000))
    return stack, amount


def row_parts(row, ops: dict[str, int]):
    if not isinstance(row, list) or not row:
        return None
    if isinstance(row[0], str):
        if len(row) < 4:
            return None
        sig_s, pk_s, flags_s = row[0], row[1], row[2]
        witness, amount = [], 0
    elif isinstance(row[0], list):
        if len(row) < 5:
            return None
        witness, amount = witness_and_amount(row[0], ops)
        sig_s, pk_s, flags_s = row[1], row[2], row[3]
    else:
        return None
    if not isinstance(sig_s, str) or not isinstance(pk_s, str) or not isinstance(flags_s, str):
        return None
    if "#TAPROOTOUTPUT#" in pk_s:
        return None
    sig = assemble(sig_s, ops)
    pk = assemble(pk_s, ops)
    return parse_flags(flags_s), sig, pk, witness, amount


def encode_shape0(flags: int, sig: bytes, pk: bytes, witness: list[bytes], amount: int) -> bytes | None:
    if amount < 0 or len(sig) > MAX_PUSH or len(pk) > MAX_PUSH or len(witness) > MAX_WIT:
        return None
    if any(len(item) > MAX_PUSH for item in witness):
        return None
    out = bytearray([0xFE])
    out += (flags & 0xFFFFFFFF).to_bytes(4, "little")
    out.append(0)
    out += (1).to_bytes(4, "little", signed=True)
    out += (0xFFFFFFFF).to_bytes(4, "little")
    out += (0).to_bytes(4, "little")
    out += int(amount).to_bytes(8, "little", signed=True)
    out += len(sig).to_bytes(2, "little") + sig
    out += len(pk).to_bytes(2, "little") + pk
    out += len(witness).to_bytes(2, "little")
    for item in witness:
        out += len(item).to_bytes(2, "little") + item
    if len(out) > MAX_LEN:
        return None
    return bytes(out)


def select_rows(rows: list, ops: dict[str, int], limit: int) -> list[bytes]:
    preferred: list[bytes] = []
    rest: list[bytes] = []
    for row in rows:
        blob = json.dumps(row)
        try:
            parts = row_parts(row, ops)
        except ValueError:
            continue
        if parts is None:
            continue
        flags, sig, pk, witness, amount = parts
        encoded = encode_shape0(flags, sig, pk, witness, amount)
        if encoded is None:
            continue
        if any(name in blob for name in PREFERRED):
            preferred.append(encoded)
        else:
            rest.append(encoded)
    chosen = preferred + rest
    return chosen[:limit]


def main(argv: list[str]) -> int:
    if len(argv) not in (3, 4):
        print(
            "usage: script-kernel-seeds.py SCRIPT_TESTS_JSON DEST_DIR [LIMIT]",
            file=sys.stderr,
        )
        return 2
    src = Path(argv[1])
    dest = Path(argv[2])
    limit = int(argv[3]) if len(argv) == 4 else 32
    ops = opcode_map(OPCODE_RS)
    rows = json.loads(src.read_text())
    if not isinstance(rows, list):
        raise SystemExit("script_tests root is not an array")
    seeds = select_rows(rows, ops, limit)
    dest.mkdir(parents=True, exist_ok=True)
    for i, seed in enumerate(seeds):
        (dest / f"core_script_{i:02d}.bin").write_bytes(seed)
    print(f"script-kernel-seeds: wrote {len(seeds)} -> {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
