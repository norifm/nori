#!/usr/bin/env python3
"""Resolve one CLI peer's announced LAN door, as the emulator cannot browse host multicast."""
import json
import os
import re
import queue
import selectors
import shlex
import sqlite3
import socket
import subprocess
import sys
import time


def lines(command, deadline):
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    pending = b""
    try:
        while time.monotonic() < deadline:
            if not selector.select(max(0, deadline - time.monotonic())):
                break
            chunk = os.read(process.stdout.fileno(), 65536)
            if not chunk:
                if pending:
                    yield pending.decode()
                break
            pending += chunk
            while b"\n" in pending:
                line, pending = pending.split(b"\n", 1)
                yield line.decode()
    finally:
        selector.close()
        process.terminate()
        process.wait()
        process.stdout.close()


def records(device, deadline):
    if sys.platform == "darwin":
        port = None
        for line in lines(["dns-sd", "-L", f"nori-{device}", "_nori._tcp", "local."], deadline):
            match = re.search(r"local\.:([0-9]+)", line)
            if match:
                port = int(match[1])
            txt = dict(word.split("=", 1) for word in shlex.split(line) if "=" in word)
            yield port, txt
        return

    from zeroconf import ServiceBrowser, ServiceStateChange, Zeroconf
    found = queue.Queue()
    zc = Zeroconf()
    browser = None

    def seen(zeroconf, service_type, name, state_change):
        if name != f"nori-{device}._nori._tcp.local." or state_change == ServiceStateChange.Removed:
            return
        remaining = deadline - time.monotonic()
        if remaining > 0:
            info = zeroconf.get_service_info(service_type, name, timeout=max(1, int(remaining * 1000)))
            if info:
                found.put((info.port, {k.decode(): v.decode() for k, v in info.properties.items() if v is not None}))

    try:
        browser = ServiceBrowser(zc, "_nori._tcp.local.", handlers=[seen])
        while (remaining := deadline - time.monotonic()) > 0:
            try:
                yield found.get(timeout=remaining)
            except queue.Empty:
                break
    finally:
        if browser:
            browser.cancel()
        zc.close()


def resolve(database, timeout=15):
    with sqlite3.connect(database) as db:
        row = db.execute("SELECT value FROM app_kv WHERE key='remoteDeviceId.terminal.0'").fetchone()
    if not row:
        raise RuntimeError("the CLI peer has not created a remote device")
    device = row[0]
    deadline = time.monotonic() + timeout
    for port, txt in records(device, deadline):
        if port and txt.get("id") == device and txt.get("kind") == "terminal":
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                    pass
            except OSError:
                continue
            return {"name": txt["name"], "door": f"10.0.2.2|{port}|" + ";".join(f"{k}={v}" for k, v in txt.items())}
    raise RuntimeError("the CLI peer did not announce its LAN door")


if __name__ == "__main__":
    try:
        print(json.dumps(resolve(sys.argv[1])))
    except (RuntimeError, sqlite3.Error, ImportError) as error:
        sys.exit(str(error))
