#!/usr/bin/env python3
"""Drive a test-control desktop build through its private Unix socket."""
import argparse
import json
import os
import socket

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--socket", default=os.environ.get("NORI_DESKTOP_SOCKET"))
parser.add_argument("command", choices=["state", "open", "do", "panel", "play_album", "find", "new_playlist", "tab", "shot", "profiles", "login", "setup_answer", "setup_back", "appearance", "click", "welcome_pick", "welcome_continue"])
parser.add_argument("arg", nargs="*")
args = parser.parse_args()
if not args.socket:
    parser.error("set NORI_DESKTOP_SOCKET or pass --socket")
bare = ("state", "profiles", "setup_back", "welcome_continue")
if args.command == "login" and not 3 <= len(args.arg) <= 6 or args.command != "login" and len(args.arg) != (0 if args.command in bare else 2 if args.command == "click" else 1):
    parser.error("state, profiles, setup_back and welcome_continue take no argument; login takes a URL, a user and a password, then optionally a proxy, its user and its password; click x and y; the others one")
request = {"command": args.command}
if args.command == "login":
    request["arg"] = dict(zip(("url", "user", "password", "proxy", "proxy_user", "proxy_password"), args.arg))
elif args.command == "click":
    request["arg"] = [float(a) for a in args.arg]
elif args.arg:
    arg = args.arg[0]
    request["arg"] = int(arg) if args.command in ("tab", "setup_answer", "appearance", "welcome_pick") else os.path.abspath(arg) if args.command == "shot" else arg
try:
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(5)
        client.connect(args.socket)
        client.sendall((json.dumps(request) + "\n").encode())
        with client.makefile() as reply:
            state = json.loads(reply.readline())
    print(json.dumps(state))
    if "error" in state:
        raise SystemExit(1)
except (OSError, ValueError) as error:
    parser.exit(1, f"desktop control: {error}\n")
