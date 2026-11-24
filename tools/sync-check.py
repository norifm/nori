#!/usr/bin/env python3
"""Pairing and profile sync against a real sync server (the self-hosted program of the neighbouring
nori-sync checkout, or NORI_SYNC_ROOT), with nori-cli --sync instances that have no screen.

    tools/sync-check.sh                      # builds what it needs, then runs this
    python3 tools/sync-check.py NORI_CLI     # with a nori-cli already built

It starts the server on a free local port, pairs two devices through it, syncs a profile both ways, stops
the server and sees both come back when it starts again, and makes a third device meet a full group
room, which it shows as a refusal. With --android SERIAL it then pairs the app on that device (its
debug build, driven by tools/app.sh) into the group and syncs a profile with it. It stops everything it
started.
"""
import os
import queue
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
passed, failed = 0, 0


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_until(what, secs, cond):
    """cond() until it gives something true, for at most secs: that value, or None."""
    end = time.monotonic() + secs
    while True:
        v = cond()
        if v:
            return v
        if time.monotonic() >= end:
            print(f"     (waited {secs}s for {what})", file=sys.stderr)
            return None
        time.sleep(0.2)


def check(name, ok):
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {name}")
    else:
        failed += 1
        print(f"  FAIL  {name}")


class Server:
    """The self-hosted sync server, with the limits the check needs."""

    def __init__(self, binary, port, data, members):
        self.binary, self.port, self.data, self.members, self.proc = binary, port, data, members, None

    def start(self):
        env = dict(os.environ, NORI_SYNC_BIND="127.0.0.1", NORI_SYNC_PORT=str(self.port), NORI_SYNC_DATA=str(self.data),
                   NORI_SYNC_MAX_MEMBERS=str(self.members), NORI_SYNC_ROOMS_PER_MINUTE="1000")
        self.proc = subprocess.Popen([self.binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
        up = wait_until("the sync server", 30, lambda: self.answers())
        if not up:
            sys.exit("the sync server did not start")

    def answers(self):
        try:
            with socket.create_connection(("127.0.0.1", self.port), timeout=1) as s:
                s.sendall(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                return b"nori-sync v1" in s.recv(4096)
        except OSError:
            return False

    def stop(self):
        if self.proc:
            self.proc.terminate()
            self.proc.wait(10)
            self.proc = None


class Device:
    """One nori-cli --sync, its answers read as they come."""

    def __init__(self, cli, data, name):
        self.name = name
        self.proc = subprocess.Popen([cli, "--sync", "--data", str(data), "--name", name], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.answers, self.told = queue.Queue(), []
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.proc.stdout:
            line = line.rstrip("\n")
            if line.startswith("told "):
                self.told.append(line[5:])
            else:
                self.answers.put(line)

    def ask(self, command):
        self.proc.stdin.write(command + "\n")
        self.proc.stdin.flush()
        try:
            return self.answers.get(timeout=20)
        except queue.Empty:
            sys.exit(f"{self.name} did not answer {command}")

    def state(self):
        return self.ask("state").split(" ", 1)[1]

    def until(self, what, secs, command, pattern):
        """Asks command until its answer matches pattern; the match or None."""
        return wait_until(f"{self.name}: {what}", secs, lambda: re.search(pattern, self.ask(command)))

    def quit(self):
        if self.proc.poll() is not None:
            return
        try:
            self.proc.stdin.write("quit\n")
            self.proc.stdin.flush()
            self.proc.wait(10)
        except (OSError, subprocess.TimeoutExpired):
            self.proc.kill()


def pair(old, new):
    """new shows a code, old takes it and confirms; whether both say so."""
    shown = re.search(r"^pairing Code (\S+)", new.ask("code")) or new.until("a code", 20, "pairing", r"^pairing Code (\S+)")
    if not shown:
        return False
    old.ask(f"take {shown.group(1)}")
    asked = old.until("what to confirm", 20, "pairing", r"Confirm")
    if not asked:
        return False
    old.ask("confirm keep")
    return bool(new.until("paired", 20, "pairing", r"^pairing Paired")) and "Sent" in old.ask("pairing")


def main():
    args = sys.argv[1:]
    android = None
    if "--android" in args:
        at = args.index("--android")
        android = args[at + 1]
        del args[at:at + 2]
    if len(args) != 2:
        sys.exit("usage: sync-check.py NORI_CLI SYNC_SERVER_BINARY [--android SERIAL]")
    cli, server_binary = args
    work = Path(tempfile.mkdtemp(prefix="nori-sync-check."))
    port = free_port()
    server = Server(server_binary, port, work / "server", members=2)
    devices = []
    try:
        server.start()
        address = f"http://127.0.0.1:{port}"
        pixel, mac = Device(cli, work / "pixel", "Pixel"), Device(cli, work / "mac", "Mac")
        devices += [pixel, mac]
        for d in devices:
            d.ask(f"server {address}")
        pixel.ask("add Home http://music.example admin secret")
        check("two devices pair through the server", pair(pixel, mac))
        check("both are connected", pixel.until("connected", 20, "state", "Connected") and mac.until("connected", 20, "state", "Connected"))
        check("the new device has the profile", mac.until("the profile", 20, "profiles", r"^profiles Home$"))
        check("each lists the other online", pixel.until("the Mac online", 20, "devices", r"Mac:online") and mac.until("the Pixel online", 20, "devices", r"Pixel:online"))
        mac.ask("rename Home Music")
        check("a change on the new device reaches the first", pixel.until("the rename", 20, "profiles", r"^profiles Music$"))
        pixel.ask("add Work http://work.example me pw")
        check("and one on the first reaches the new one", mac.until("the new profile", 20, "profiles", r"^profiles Music,Work$"))
        server.stop()
        check("a dropped connection waits quietly", pixel.until("waiting", 20, "state", r"Waiting \{ refused: None") and mac.until("waiting", 20, "state", r"Waiting \{ refused: None"))
        server.start()
        check("and comes back by itself", pixel.until("back", 60, "state", "Connected") and mac.until("back", 60, "state", "Connected"))
        check("no drop was told as an event", not any("Refused" in t for t in pixel.told + mac.told))
        mac.ask("remove Work")
        check("after it, sync goes on", pixel.until("the removal", 20, "profiles", r"^profiles Music$"))
        ipad = Device(cli, work / "ipad", "iPad")
        devices.append(ipad)
        ipad.ask(f"server {address}")
        check("a third device pairs", pair(pixel, ipad))
        check("and shows the full group room as a refusal", ipad.until("refused", 20, "state", r"Waiting \{ refused: Some\(Limit\)"))
        if android:
            check_android(android, pixel, address)
    finally:
        for d in devices:
            d.quit()
        server.stop()
        shutil.rmtree(work, ignore_errors=True)
    print(f"sync check: {passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


def app(serial, *words):
    out = subprocess.run([str(HERE / "app.sh"), *words], env=dict(os.environ, ANDROID_SERIAL=serial), capture_output=True, text=True, timeout=60)
    return out.stdout.strip()


def check_android(serial, pixel, address):
    """The app on `serial` pairs into pixel's group through the server on this Mac, and syncs."""
    # The emulator reaches this Mac's 127.0.0.1 as 10.0.2.2; the Mac keeps its own address.
    app(serial, "sync", "server", address.replace("127.0.0.1", "10.0.2.2"))
    app(serial, "sync", "code")
    code = wait_until("the app's code", 30, lambda: re.search(r"(nori://pair#\S+)", app(serial, "sync", "pairing")))
    check("the app shows a code", code)
    if not code:
        return
    pixel.ask(f"take {code.group(1)}")
    asked = pixel.until("what to confirm", 30, "pairing", r"Confirm")
    pixel.ask("confirm keep")
    check("the app pairs into the group", asked and wait_until("paired", 30, lambda: "Paired" in app(serial, "sync", "pairing")))
    check("the app is connected", wait_until("connected", 30, lambda: "Connected" in app(serial, "sync", "state")))
    check("the app has the group's profiles", wait_until("profiles", 30, lambda: "Music" in app(serial, "sync", "profiles")))
    pixel.ask("rename Music Records")
    check("a change reaches the app", wait_until("the rename", 30, lambda: "Records" in app(serial, "sync", "profiles")))


if __name__ == "__main__":
    main()
