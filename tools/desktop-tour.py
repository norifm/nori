#!/usr/bin/env python3
"""Walks a running desktop client (built with `--features test-control`, started with
`--control-socket PATH`) through every page, panel and the full player, `rounds` times, with `pause`
seconds on each: a fixed workload for profiling the interface (tools/flame.py --for).

  tools/desktop-tour.py SOCKET [rounds=3] [pause=0.5] [album id: played first, to profile while music plays]
"""
import json
import socket
import sys
import time

STEPS = (
    [("open", v) for v in ("home", "albums", "artists", "playlists", "songs", "search", "settings", "equalizer")]
    + [("panel", p) for p in ("queue", "lyrics", "devices", "none")]
    + [("open", "fullscreen"), ("do", "close_fullscreen")]
)


def send(path, command, arg):
    with socket.socket(socket.AF_UNIX) as s:
        s.connect(path)
        s.sendall((json.dumps({"command": command, "arg": arg}) + "\n").encode())
        return s.makefile().readline()


def main():
    path = sys.argv[1]
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    pause = float(sys.argv[3]) if len(sys.argv) > 3 else 0.5
    album = sys.argv[4] if len(sys.argv) > 4 else None
    deadline = time.time() + 30
    while True:
        try:
            send(path, "state", None)
            break
        except OSError:
            if time.time() > deadline:
                sys.exit(f"no desktop client answers on {path}")
            time.sleep(0.2)
    if album:
        send(path, "play_album", album)
    for _ in range(rounds):
        for command, arg in STEPS:
            reply = send(path, command, arg)
            if '"error"' in reply:
                sys.exit(f"{command} {arg}: {reply.strip()}")
            time.sleep(pause)


if __name__ == "__main__":
    main()
