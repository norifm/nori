#!/usr/bin/env python3
"""Pairs a terminal peer's data folder with the app on a device, for the device checks that use the
terminal as the other device (remote control reaches only paired devices).

    python3 tools/pair-peer.py NORI_CLI DATA URL USER PASSWORD

It starts the self-hosted sync server of the neighbouring nori-sync checkout (NORI_SYNC_ROOT names
another, tools/sync-check.sh builds it) on a free port, gives the device that port and the URL's through
adb reverse, adds the profile URL USER PASSWORD to the peer, and pairs the app into the peer's group. The
profile syncs to the app. It prints the server's pid, its port and the URL's port, for the caller to stop
the server and remove the ports: `kill PID; adb reverse --remove tcp:PORT`.
"""
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = __import__("importlib.util").util.spec_from_file_location("sync_check", HERE / "sync-check.py")
sync_check = __import__("importlib.util").util.module_from_spec(spec)
spec.loader.exec_module(sync_check)


def main():
    cli, data, url, user, password = sys.argv[1:6]
    serial = os.environ.get("ANDROID_SERIAL", "emulator-5554")
    common = subprocess.run(["git", "-C", str(HERE), "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True, text=True).stdout.strip()
    root = Path(os.environ.get("NORI_SYNC_ROOT", Path(common).parent.parent / "nori-sync"))
    server = sync_check.Server(str(root / "target/release/nori-sync-server"), sync_check.free_port(), Path(data) / "sync-server", members=20)
    server.start()
    address = f"http://127.0.0.1:{server.port}"
    url_port = url.rsplit(":", 1)[1].split("/")[0]
    for port in (str(server.port), url_port):
        subprocess.run(["adb", "-s", serial, "reverse", f"tcp:{port}", f"tcp:{port}"], check=True, capture_output=True)
    peer = sync_check.Device(cli, data, "Peer")
    try:
        peer.ask(f"server {address}")
        peer.ask(f"add Peer {url} {user} {password}")
        sync_check.app(serial, "sync", "server", address)
        sync_check.app(serial, "sync", "code")
        code = sync_check.wait_until("the app's code", 30, lambda: re.search(r"(nori://pair#\S+)", sync_check.app(serial, "sync", "pairing")))
        if not code:
            sys.exit("the app showed no code")
        peer.ask(f"take {code.group(1)}")
        if not peer.until("what to confirm", 30, "pairing", r"Confirm"):
            sys.exit("the peer was not asked to confirm")
        peer.ask("confirm")
        if not sync_check.wait_until("paired", 30, lambda: "Peer" in sync_check.app(serial, "sync", "profiles")):
            sys.exit("the peer's profile did not reach the app")
    finally:
        peer.quit()
    print(server.proc.pid, server.port, url_port)


if __name__ == "__main__":
    main()
