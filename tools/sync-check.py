#!/usr/bin/env python3
"""Pairing and profile sync against a real sync server (the self-hosted program of the neighbouring
nori-sync checkout, or NORI_SYNC_ROOT), with nori-cli --sync instances that have no screen.

    tools/sync-check.sh                      # builds what it needs, then runs this
    python3 tools/sync-check.py NORI_CLI     # with a nori-cli already built

It starts the server on a free local port, pairs two devices through it, syncs a profile both ways, stops
the server and sees both come back when it starts again, and makes a third device meet a full group
room, which it shows as a refusal. Then two paired terminals with a profile of the fixture Navidrome
(tools/dev-server.sh, on localhost:4533) play and control each other: through the server, and directly
once the server is gone; a profile that does not sync offers no device. With --android SERIAL it pairs
the app on that device (its debug build, driven by tools/app.sh) into a group, syncs a profile with it,
and controls it from a terminal and back. It stops everything it started.
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
        """Asks command until its answer matches pattern; the match or None, saying the last answer."""
        last = [""]

        def matched():
            last[0] = self.ask(command)
            return re.search(pattern, last[0])
        found = wait_until(f"{self.name}: {what}", secs, matched)
        if not found:
            print(f"     ({self.name} last said: {last[0]})", file=sys.stderr)
        return found

    def quit(self):
        if self.proc.poll() is not None:
            return
        try:
            self.proc.stdin.write("quit\n")
            self.proc.stdin.flush()
            self.proc.wait(10)
        except (OSError, subprocess.TimeoutExpired):
            self.proc.kill()


FIXTURE = "http://localhost:4533"


def fixture_up():
    try:
        with socket.create_connection(("127.0.0.1", 4533), timeout=1):
            return True
    except OSError:
        return False


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
        check("a dropped connection waits quietly", pixel.until("waiting", 20, "state", r"Offline") and mac.until("waiting", 20, "state", r"Offline"))
        server.start()
        check("and comes back by itself", pixel.until("back", 60, "state", "Connected") and mac.until("back", 60, "state", "Connected"))
        check("no drop was told as an event", not any("Refused" in t for t in pixel.told + mac.told))
        mac.ask("remove Work")
        check("after it, sync goes on", pixel.until("the removal", 20, "profiles", r"^profiles Music$"))
        ipad = Device(cli, work / "ipad", "iPad")
        devices.append(ipad)
        ipad.ask(f"server {address}")
        check("a third device pairs", pair(pixel, ipad))
        check("and shows the full group room as a refusal", ipad.until("refused", 20, "state", r"Refused \{ refused: Limit"))
        for d in (pixel, mac, ipad):
            d.quit()
        server.stop()
        if not fixture_up():
            print("  NOTE  remote control needs the fixture Navidrome: tools/dev-server.sh")
        else:
            remote = Server(server_binary, free_port(), work / "server-remote", members=20)
            server = remote
            remote.start()
            address = f"http://127.0.0.1:{remote.port}"
            desk, laptop = Device(cli, work / "desk", "Desk"), Device(cli, work / "laptop", "Laptop")
            devices += [desk, laptop]
            check_remote(desk, laptop, remote, address)
            if android:
                laptop.quit()
                check_android(android, desk, remote, address, work / "desk")
    finally:
        for d in devices:
            d.quit()
        server.stop()
        shutil.rmtree(work, ignore_errors=True)
    print(f"sync check: {passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


def playing(device, where, state):
    """Whether device's player shows `where` (here, or another device's name) `state` (playing, paused)."""
    return lambda: re.match(rf"now {where} {state}", device.ask("now"))


def check_remote(desk, laptop, server, address):
    """Two paired terminals: the desk plays, the laptop sees it, controls it and moves the music to itself,
    through the server and then directly; a profile that does not sync offers no device."""
    for d in (desk, laptop):
        d.ask(f"server {address}")
    desk.ask(f"add Home {FIXTURE} admin admin")
    check("two terminals pair for remote control", pair(desk, laptop))
    check("the profile synced", laptop.until("the profile", 20, "profiles", r"^profiles Home$"))
    for d in (desk, laptop):
        check(f"{d.name} opens its profile", d.ask("open").startswith("open Home"))
    check("the desk plays", desk.ask("play Long Track").startswith("play ") and wait_until("the desk playing", 30, playing(desk, "here", "playing")))
    laptop.ask("remote watch on")
    check("the laptop sees the desk playing", laptop.until("the desk", 30, "remote devices", r"devices [^|]*:playing Long Track"))
    laptop.ask("remote send * pause")
    check("the laptop pauses it", wait_until("the desk paused", 20, playing(desk, "here", "paused")))
    laptop.ask("remote pick *")
    laptop.ask("remote watch off")
    check("and shows it as the device playing", wait_until("the mirror", 20, lambda: re.match(r"now (?!here)\S.* paused", laptop.ask("now"))))
    laptop.ask("remote send * play")
    check("plays it again", wait_until("the desk playing again", 20, playing(desk, "here", "playing")))
    laptop.ask("remote pick here")
    check("and moves the music to itself", wait_until("the laptop playing", 30, playing(laptop, "here", "playing")))
    check("the desk hands over and follows the laptop", wait_until("the desk following", 20, lambda: re.match(r"now (?!here)\S.* playing", desk.ask("now"))))
    check("on one network the devices find each other directly", desk.until("nearby", 60, "remote devices", r"\(nearby\)"))
    server.stop()
    check("both lose the server", desk.until("offline", 20, "state", r"Offline") and laptop.until("offline", 20, "state", r"Offline"))
    desk.ask("remote watch on")
    desk.ask("remote send * pause")
    check("without it the desk pauses the laptop through its door", wait_until("the laptop paused", 20, playing(laptop, "here", "paused")))
    desk.ask("remote send * play")
    check("and plays it again", wait_until("the laptop playing", 20, playing(laptop, "here", "playing")))
    desk.ask("rename Home Records")
    check("profile sync goes through the door too", laptop.until("the rename", 30, "profiles", r"^profiles Records$"))
    server.start()
    check("both come back to the server", desk.until("back", 60, "state", "Connected") and laptop.until("back", 60, "state", "Connected"))
    laptop.ask("local Records")
    check("a profile that does not sync offers no device", desk.until("the laptop gone", 30, "remote devices", r"^remote devices $"))


def app(serial, *words):
    out = subprocess.run([str(HERE / "app.sh"), *words], env=dict(os.environ, ANDROID_SERIAL=serial), capture_output=True, text=True, timeout=60)
    return out.stdout.strip()


def check_android(serial, desk, server, address, data):
    """The app on `serial`, started afresh, pairs into the desk's group through the server on this Mac,
    plays the group's profile, and the desk controls it: through the server, as the emulator hears no
    mDNS, and then directly once the desk's door is handed to it."""
    # A code carries its server, which both sides must reach at one address, and the synced profile its
    # Navidrome: the device's own ports are this Mac's.
    ports = [address.rsplit(":", 1)[1], "4533"]
    for port in ports:
        subprocess.run(["adb", "-s", serial, "reverse", f"tcp:{port}", f"tcp:{port}"], check=True, capture_output=True)
    try:
        check_app(serial, desk, server, address, data)
    finally:
        for port in ports:
            subprocess.run(["adb", "-s", serial, "reverse", "--remove", f"tcp:{port}"], capture_output=True)


def app_playing(serial, playing):
    return lambda: f'"playing":{"true" if playing else "false"}' in app(serial, "state")


def check_app(serial, desk, server, address, data):
    subprocess.run(["adb", "-s", serial, "shell", "pm", "clear", "dev.nori.music"], capture_output=True)
    app(serial, "launch")
    app(serial, "sync", "server", address)
    app(serial, "sync", "code")
    code = wait_until("the app's code", 30, lambda: re.search(r"(nori://pair#\S+)", app(serial, "sync", "pairing")))
    check("the app shows a code", code)
    if not code:
        return
    desk.ask(f"take {code.group(1)}")
    asked = desk.until("what to confirm", 30, "pairing", r"Confirm")
    desk.ask("confirm")
    check("the app pairs into the group", asked and wait_until("paired", 30, lambda: "Paired" in app(serial, "sync", "pairing")))
    check("the app is connected", wait_until("connected", 30, lambda: "Connected" in app(serial, "sync", "state")))
    check("the app has the group's profile", wait_until("profiles", 30, lambda: "Records" in app(serial, "sync", "profiles")))
    app(serial, "set", "remoteControl", "true")
    app(serial, "play", "search:Long Track 01")
    check("the app plays the synced profile", wait_until("the app playing", 60, app_playing(serial, True)))
    desk.ask("remote watch on")
    check("the desk sees the app playing, through the server", desk.until("the app", 30, "remote devices", r"devices (?![^|]*nearby)[^|]*:playing Long Track"))
    desk.ask("remote send * pause")
    check("the desk pauses the app", wait_until("the app paused", 20, app_playing(serial, False)))
    desk.ask("remote send * play")
    check("and plays it again", wait_until("the app playing again", 20, app_playing(serial, True)))
    desk.ask("remote pick *")
    desk.ask("remote watch off")
    desk.ask("remote pick here")
    check("the desk moves the music to itself", wait_until("the desk playing", 30, playing(desk, "here", "playing")))
    check("and the app hands it over, following the desk", wait_until("the app following", 20, lambda: re.search(r'"playingOn":"[^"]+"', app(serial, "state"))))
    # The emulator hears no multicast from here: the app is handed the desk's door as mDNS has it.
    resolved = subprocess.run([sys.executable, str(HERE / "mdns-door.py"), str(data / "nori.db")], capture_output=True, text=True)
    check("the desk announces its door", resolved.returncode == 0)
    if resolved.returncode != 0:
        return
    door = __import__("json").loads(resolved.stdout)["door"]
    server.stop()
    app(serial, "remote", "watch", "on")
    app(serial, "remote", "found", door)
    check("without the server the app finds the desk at its door", wait_until("nearby", 30, lambda: "(nearby): playing" in app(serial, "remote", "devices")))
    app(serial, "remote", "watch", "off")
    app(serial, "remote", "pick", "here")
    check("and takes the music back through it", wait_until("the app playing", 30, app_playing(serial, True)))
    check("the desk hands it over", wait_until("the desk following", 20, lambda: not re.match(r"now here playing", desk.ask("now"))))
    server.start()


if __name__ == "__main__":
    main()
