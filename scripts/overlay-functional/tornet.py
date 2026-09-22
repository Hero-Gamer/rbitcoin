#!/usr/bin/env python3
"""Private TestingTorNetwork (chutney basic-min equivalent). Never bootstraps public Tor."""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

AUTH_COUNT = 3
VOTE = 10


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, text=True, **kw)


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def tor_gencert(datadir: Path, dirport: int) -> str:
    keys = datadir / "keys"
    keys.mkdir(parents=True, exist_ok=True)
    ident = keys / "authority_identity_key"
    signing = keys / "authority_signing_key"
    cert = keys / "authority_certificate"
    run(
        [
            "tor-gencert",
            "--create-identity-key",
            "--passphrase-fd",
            "0",
            "-m",
            "12",
            "-a",
            f"127.0.0.1:{dirport}",
            "-i",
            str(ident),
            "-s",
            str(signing),
            "-c",
            str(cert),
        ],
        input="\n",
    )
    v3ident = None
    for line in cert.read_text().splitlines():
        if line.lower().startswith("fingerprint"):
            v3ident = line.split()[-1].replace(" ", "")
            break
    if not v3ident:
        raise SystemExit(f"no v3ident in {cert}")
    return v3ident


def list_fingerprint(datadir: Path, nick: str) -> str:
    dummy = datadir / "fp.torrc"
    write(
        dummy,
        "\n".join(
            [
                f"DataDirectory {datadir}",
                f"Nickname {nick}",
                "ORPort 127.0.0.1:1",
                "DirPort 0",
                "SocksPort 0",
                "TestingTorNetwork 1",
                'DirAuthority x orport=1 v3ident=0000000000000000000000000000000000000000 127.0.0.1:1 0000000000000000000000000000000000000000',
            ]
        )
        + "\n",
    )
    out = run(
        ["tor", "--quiet", "-f", str(dummy), "--list-fingerprint"],
        capture_output=True,
    ).stdout.strip()
    dummy.unlink(missing_ok=True)
    parts = out.split()
    fp = "".join(parts[1:]) if len(parts) > 1 else parts[-1]
    return fp.replace(" ", "")


def dirauth_lines(auths: list[dict]) -> str:
    lines = []
    for a in auths:
        lines.append(
            f"DirAuthority {a['nick']} orport={a['orport']} v3ident={a['v3ident']} "
            f"127.0.0.1:{a['dirport']} {a['fp']}"
        )
    return "\n".join(lines)


def common_torrc(datadir: Path, nick: str, auths: str) -> str:
    return "\n".join(
        [
            "TestingTorNetwork 1",
            "AssumeReachable 1",
            "TestingAuthDirTimeToLearnReachability 0",
            "TestingMinExitFlagThreshold 0",
            f"TestingV3AuthInitialVotingInterval {VOTE}",
            "TestingV3AuthInitialVoteDelay 2",
            "TestingV3AuthInitialDistDelay 2",
            # TestingTorNetwork's ongoing vote is 5 minutes; keep the tiny-net cadence.
            f"V3AuthVotingInterval {VOTE}",
            "V3AuthVoteDelay 2",
            "V3AuthDistDelay 2",
            "TestingDirAuthVoteHSDir *",
            "TestingDirAuthVoteGuard *",
            "TestingDirAuthVoteExit *",
            "MinUptimeHidServDirectoryV2 0",
            "ConfluxEnabled 0",
            "LearnCircuitBuildTimeout 0",
            "CircuitBuildTimeout 10",
            "PathsNeededToBuildCircuits 0.25",
            "DataDirectory {0}".format(datadir),
            f"Nickname {nick}",
            f"PidFile {datadir / 'tor.pid'}",
            f"Log notice file {datadir / 'notice.log'}",
            "SafeLogging 0",
            "ProtocolWarnings 1",
            "ShutdownWaitLength 0",
            "DisableDebuggerAttachment 0",
            "FetchHidServDescriptors 1",
            "HiddenServiceStatistics 0",
            "ServerDNSDetectHijacking 0",
            "ServerDNSTestAddresses",
            auths,
            "RunAsDaemon 1",
        ]
    )


def start_tor(torrc: Path) -> None:
    run(["tor", "-f", str(torrc)])


def wait_log(log: Path, needles: tuple[str, ...], seconds: int) -> None:
    deadline = time.time() + seconds
    while time.time() < deadline:
        if log.is_file():
            text = log.read_text(errors="replace")
            if any(n in text for n in needles):
                return
        time.sleep(0.4)
    extra = ""
    if log.is_file():
        extra = "\n".join(log.read_text(errors="replace").splitlines()[-25:])
    raise SystemExit(f"bootstrap timeout: {log}\n{extra}")


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: tornet.py DIR", file=sys.stderr)
        return 2
    root = Path(sys.argv[1])
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    env_path = root / "env"

    auths = []
    for i in range(AUTH_COUNT):
        nick = f"auth{i}"
        datadir = root / nick
        datadir.mkdir()
        orport = free_port()
        dirport = free_port()
        v3ident = tor_gencert(datadir, dirport)
        fp = list_fingerprint(datadir, nick)
        auths.append(
            {
                "nick": nick,
                "datadir": datadir,
                "orport": orport,
                "dirport": dirport,
                "v3ident": v3ident,
                "fp": fp,
            }
        )

    da = dirauth_lines(auths)
    for a in auths:
        extra = "\n".join(
            [
                "SocksPort 0",
                f"ORPort {a['orport']} IPv4Only",
                f"DirPort {a['dirport']} IPv4Only",
                "Address 127.0.0.1",
                "AuthoritativeDirectory 1",
                "V3AuthoritativeDirectory 1",
                "ContactInfo auth@test.test",
                "ExitRelay 1",
                "ExitPolicy accept *:*",
                "IPv6Exit 0",
            ]
        )
        torrc = a["datadir"] / "torrc"
        write(torrc, common_torrc(a["datadir"], a["nick"], da) + "\n" + extra + "\n")
        start_tor(torrc)

    relays = []
    for i in range(3):
        nick = f"relay{i}"
        datadir = root / nick
        datadir.mkdir()
        orport = free_port()
        fp = list_fingerprint(datadir, nick)
        relays.append({"nick": nick, "datadir": datadir, "orport": orport, "fp": fp})
        extra = "\n".join(
            [
                "SocksPort 0",
                f"ORPort {orport} IPv4Only",
                "DirPort 0",
                "Address 127.0.0.1",
                "ExitRelay 1",
                "ExitPolicy accept *:*",
            ]
        )
        torrc = datadir / "torrc"
        write(torrc, common_torrc(datadir, nick, da) + "\n" + extra + "\n")
        start_tor(torrc)

    for a in auths:
        wait_log(
            a["datadir"] / "notice.log",
            ("Bootstrapped 100%", "Published ns consensus"),
            90,
        )
    for r in relays:
        wait_log(r["datadir"] / "notice.log", ("Bootstrapped 100%",), 90)

    client = root / "client"
    client.mkdir()
    socks = free_port()
    control = free_port()
    cookie = client / "control_auth_cookie"
    extra = "\n".join(
        [
            f"SocksPort 127.0.0.1:{socks}",
            f"ControlPort 127.0.0.1:{control}",
            "CookieAuthentication 1",
            f"CookieAuthFile {cookie}",
            "ClientOnly 1",
            "ORPort 0",
            "DirPort 0",
            "SocksPolicy accept 127.0.0.0/8",
        ]
    )
    torrc = client / "torrc"
    write(torrc, common_torrc(client, "client", da) + "\n" + extra + "\n")
    start_tor(torrc)
    wait_log(client / "notice.log", ("Bootstrapped 100%",), 120)

    if not cookie.is_file():
        raise SystemExit("control cookie missing")
    write(
        env_path,
        "\n".join(
            [
                f"OVERLAY_TOR_SOCKS=127.0.0.1:{socks}",
                f"OVERLAY_TOR_CONTROL=127.0.0.1:{control}",
                f"OVERLAY_TOR_COOKIE={cookie}",
            ]
        )
        + "\n",
    )
    print(env_path.read_text(), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
